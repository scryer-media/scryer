import * as React from "react";
import { useClient } from "urql";
import { librariesQuery, titleHistoryQuery } from "@/lib/graphql/queries";
import { retryImportMutation } from "@/lib/graphql/mutations";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRecord, TitleHistoryEvent, TitleHistoryPage, TitleRecord } from "@/lib/types";
import { WANTED_HISTORY_FILTERS } from "@/components/common/title-history-event-meta";
import { TitleHistoryView } from "@/components/views/title-history-view";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";

const PAGE_SIZE = 50;
export function TitleHistoryContainer({
  showRetryActions = true,
}: {
  showRetryActions?: boolean;
}) {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [totalCount, setTotalCount] = React.useState(0);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [activeFilters, setActiveFilters] = React.useState<string[]>([]);
  const [page, setPage] = React.useState(0);
  const [selectedTitle, setSelectedTitle] = React.useState<TitleRecord | null>(null);
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);

  const selectedEventTypes = React.useMemo(
    () => (activeFilters.length > 0 ? activeFilters : [...WANTED_HISTORY_FILTERS]),
    [activeFilters],
  );
  const offset = page * PAGE_SIZE;

  const fetchHistory = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await client
        .query<{ titleHistory: TitleHistoryPage }>(titleHistoryQuery, {
          filter: {
            // Chips use the lowercase display keys; TitleHistoryEventTypeValue
            // members are their exact uppercase.
            eventTypes: selectedEventTypes.map((value) => value.toUpperCase()),
            titleIds: selectedTitle ? [selectedTitle.id] : null,
            libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
            groupByEvent: true,
            limit: PAGE_SIZE,
            offset,
          },
        })
        .toPromise();

      if (result.error) {
        throw result.error;
      }

      setEvents(result.data?.titleHistory.items ?? []);
      setTotalCount(result.data?.titleHistory.totalCount ?? 0);
    } catch (fetchError) {
      setError(
        fetchError instanceof Error ? fetchError.message : t("status.failedToLoad"),
      );
      setEvents([]);
      setTotalCount(0);
    } finally {
      setLoading(false);
    }
  }, [client, offset, selectedEventTypes, selectedLibraryIds, selectedTitle, t]);

  React.useEffect(() => {
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
      .catch((fetchError) => {
        if (!cancelled) {
          setGlobalStatus(
            fetchError instanceof Error ? fetchError.message : t("status.failedToLoad"),
          );
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
  }, [client, setGlobalStatus, t]);

  React.useEffect(() => {
    void fetchHistory();
  }, [fetchHistory]);

  const toggleFilter = React.useCallback((eventType: string) => {
    setPage(0);
    setActiveFilters((prev) =>
      prev.includes(eventType)
        ? prev.filter((current) => current !== eventType)
        : [...prev, eventType],
    );
  }, []);

  const clearFilters = React.useCallback(() => {
    setPage(0);
    setActiveFilters([]);
  }, []);

  const handleSelectedTitleChange = React.useCallback((title: TitleRecord | null) => {
    setPage(0);
    setSelectedTitle(title);
  }, []);

  const handleSelectedLibraryIdsChange = React.useCallback((libraryIds: string[]) => {
    setPage(0);
    setSelectedLibraryIds(libraryIds);
  }, []);

  const handlePreviousPage = React.useCallback(() => {
    setPage((current) => Math.max(0, current - 1));
  }, []);

  const handleNextPage = React.useCallback(() => {
    setPage((current) => current + 1);
  }, []);

  const handleRetry = React.useCallback(
    async (importId: string, password?: string) => {
      try {
        const { error: retryError } = await client
          .mutation(retryImportMutation, {
            input: { importId, password: password || null },
          })
          .toPromise();

        if (retryError) {
          throw retryError;
        }

        setGlobalStatus(t("importHistory.retrySuccess"));
        await fetchHistory();
      } catch (retryError) {
        setGlobalStatus(
          retryError instanceof Error ? retryError.message : t("status.apiError"),
        );
      }
    },
    [client, fetchHistory, setGlobalStatus, t],
  );

  return (
    <TitleHistoryView
      events={events}
      totalCount={totalCount}
      loading={loading}
      error={error}
      activeFilters={activeFilters}
      availableFilters={[...WANTED_HISTORY_FILTERS]}
      selectedTitle={selectedTitle}
      libraries={libraries}
      librariesLoading={librariesLoading}
      selectedLibraryIds={selectedLibraryIds}
      currentPage={page}
      pageSize={PAGE_SIZE}
      onSelectedTitleChange={handleSelectedTitleChange}
      onSelectedLibraryIdsChange={handleSelectedLibraryIdsChange}
      onToggleFilter={toggleFilter}
      onClearFilters={clearFilters}
      onPreviousPage={handlePreviousPage}
      onNextPage={handleNextPage}
      onRetry={showRetryActions ? handleRetry : undefined}
      hasPreviousPage={page > 0}
      hasNextPage={offset + events.length < totalCount}
    />
  );
}

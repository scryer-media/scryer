import * as React from "react";
import { useClient } from "urql";
import { titleHistoryQuery } from "@/lib/graphql/queries";
import type { TitleHistoryEvent, TitleHistoryPage } from "@/lib/types";
import { TitleHistoryView } from "@/components/views/title-history-view";

const PAGE_SIZE = 50;

const EVENT_TYPE_FILTERS = [
  "grabbed",
  "download_completed",
  "imported",
  "import_failed",
  "import_skipped",
  "file_deleted",
  "file_renamed",
  "download_ignored",
] as const;

export function TitleHistoryContainer() {
  const client = useClient();
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [totalCount, setTotalCount] = React.useState(0);
  const [loading, setLoading] = React.useState(true);
  const [activeFilters, setActiveFilters] = React.useState<string[]>([]);
  const [offset, setOffset] = React.useState(0);

  const fetchHistory = React.useCallback(
    async (eventTypes: string[], pageOffset: number, append: boolean) => {
      setLoading(true);
      try {
        const result = await client
          .query<{ titleHistory: TitleHistoryPage }>(titleHistoryQuery, {
            filter: {
              eventTypes: eventTypes.length > 0 ? eventTypes : null,
              limit: PAGE_SIZE,
              offset: pageOffset,
            },
          })
          .toPromise();

        if (result.data?.titleHistory) {
          const page = result.data.titleHistory;
          setEvents((prev) =>
            append ? [...prev, ...page.records] : page.records,
          );
          setTotalCount(page.totalCount);
        }
      } finally {
        setLoading(false);
      }
    },
    [client],
  );

  React.useEffect(() => {
    setOffset(0);
    void fetchHistory(activeFilters, 0, false);
  }, [activeFilters, fetchHistory]);

  const loadMore = React.useCallback(() => {
    const nextOffset = offset + PAGE_SIZE;
    setOffset(nextOffset);
    void fetchHistory(activeFilters, nextOffset, true);
  }, [offset, activeFilters, fetchHistory]);

  const toggleFilter = React.useCallback((eventType: string) => {
    setActiveFilters((prev) =>
      prev.includes(eventType)
        ? prev.filter((f) => f !== eventType)
        : [...prev, eventType],
    );
  }, []);

  const clearFilters = React.useCallback(() => {
    setActiveFilters([]);
  }, []);

  return (
    <TitleHistoryView
      events={events}
      totalCount={totalCount}
      loading={loading}
      activeFilters={activeFilters}
      availableFilters={EVENT_TYPE_FILTERS as unknown as string[]}
      onToggleFilter={toggleFilter}
      onClearFilters={clearFilters}
      onLoadMore={loadMore}
      hasMore={events.length < totalCount}
    />
  );
}

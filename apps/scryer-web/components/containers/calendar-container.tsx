import { lazy, memo, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";
import { calendarEpisodesQuery, librariesQuery } from "@/lib/graphql/queries";
import { facetById } from "@/lib/facets/registry";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { CalendarEpisodeItem } from "@/components/views/calendar-view";
import type { LibraryRecord } from "@/lib/types";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";

const CalendarView = lazy(() =>
  import("@/components/views/calendar-view").then((m) => ({ default: m.CalendarView })),
);

type CalendarContainerProps = {
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

function sameStringArray(left: string[], right: string[]) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

export const CalendarContainer = memo(function CalendarContainer({
  onOpenOverview,
}: CalendarContainerProps) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [calendarEpisodes, setCalendarEpisodes] = useState<CalendarEpisodeItem[]>([]);
  const [calendarLoading, setCalendarLoading] = useState(false);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = useState<string[]>([]);
  const lastCalendarRangeRef = useRef<{ start: string; end: string } | null>(null);

  useEffect(() => {
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
        setSelectedLibraryIds((current) => {
          const normalized = normalizeLibraryFilterSelection(current, nextLibraries);
          return sameStringArray(current, normalized) ? current : normalized;
        });
      })
      .catch((error) => {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : t("status.failedToLoad");
          setGlobalStatus(message, { level: "ERROR" });
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

  const refreshCalendar = useCallback(
    async (start: string, end: string) => {
      lastCalendarRangeRef.current = { start, end };
      setCalendarLoading(true);
      try {
        const { data, error } = await client
          .query(calendarEpisodesQuery, {
            startDate: start,
            endDate: end,
            libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
          })
          .toPromise();
        if (error) throw error;
        setCalendarEpisodes(data?.calendarEpisodes ?? []);
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToLoad");
        setGlobalStatus(message, { level: "ERROR" });
      } finally {
        setCalendarLoading(false);
      }
    },
    [client, selectedLibraryIds, setGlobalStatus, t],
  );

  useEffect(() => {
    const range = lastCalendarRangeRef.current;
    if (range) {
      void refreshCalendar(range.start, range.end);
    }
  }, [refreshCalendar, selectedLibraryIds]);

  const handleCalendarEpisodeClick = useCallback(
    (episode: CalendarEpisodeItem) => {
      const facet = facetById(episode.titleFacet.toUpperCase());
      if (!facet || !onOpenOverview) return;
      onOpenOverview(
        facet.viewId as ViewId,
        {
          id: episode.titleId,
          slug: episode.titleSlug ?? null,
          libraryId: episode.libraryId,
          librarySlug: episode.librarySlug ?? null,
        },
        facet.id === "MOVIE" ? undefined : episode.id,
      );
    },
    [onOpenOverview],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain bg-transparent">
      <div className="mx-auto flex min-h-0 w-full max-w-none flex-1 flex-col px-4 py-4 pb-[calc(env(safe-area-inset-bottom)+5rem)] sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]">
        <Suspense
          fallback={
            <div className="py-6 text-sm text-[var(--scry-muted3)]">
              {t("label.loading")}
            </div>
          }
        >
          <CalendarView
            episodes={calendarEpisodes}
            loading={calendarLoading}
            libraries={libraries}
            librariesLoading={librariesLoading}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={setSelectedLibraryIds}
            onDateRangeChange={refreshCalendar}
            onEpisodeClick={handleCalendarEpisodeClick}
          />
        </Suspense>
      </div>
    </div>
  );
});

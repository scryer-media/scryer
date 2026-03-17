import * as React from "react";
import { Loader2 } from "lucide-react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { titleHistoryQuery } from "@/lib/graphql/queries";
import type { TitleHistoryEvent, TitleHistoryPage } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { HistoryEventTable } from "./history-event-table";
import {
  HistoryEventIcon,
} from "./history-event-icon";

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
];

const filterI18nKeys: Record<string, string> = {
  grabbed: "history.grabbed",
  download_completed: "history.downloadCompleted",
  imported: "history.imported",
  import_failed: "history.importFailed",
  import_skipped: "history.importSkipped",
  file_deleted: "history.fileDeleted",
  file_renamed: "history.fileRenamed",
  download_ignored: "history.downloadIgnored",
};

export function TitleHistoryModal({
  open,
  onOpenChange,
  titleId,
  titleName,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId: string;
  titleName: string;
}) {
  const client = useClient();
  const t = useTranslate();
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [totalCount, setTotalCount] = React.useState(0);
  const [loading, setLoading] = React.useState(false);
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
              titleIds: [titleId],
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
    [client, titleId],
  );

  React.useEffect(() => {
    if (open) {
      setOffset(0);
      setEvents([]);
      void fetchHistory(activeFilters, 0, false);
    }
  }, [open, activeFilters, fetchHistory]);

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

  const hasMore = events.length < totalCount;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[calc(100%-1rem)] max-w-[95vw] sm:max-w-5xl lg:max-w-6xl max-h-[85vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle>{titleName} — {t("history.title")}</DialogTitle>
        </DialogHeader>

        <div className="flex flex-wrap gap-1.5 pb-2">
          <Button
            type="button"
            size="sm"
            variant={activeFilters.length === 0 ? "default" : "secondary"}
            onClick={clearFilters}
            className="h-7 text-xs"
          >
            {t("history.allEvents")}
          </Button>
          {EVENT_TYPE_FILTERS.map((eventType) => {
            const isActive = activeFilters.includes(eventType);
            return (
              <Button
                key={eventType}
                type="button"
                size="sm"
                variant={isActive ? "default" : "secondary"}
                onClick={() => toggleFilter(eventType)}
                className="h-7 gap-1.5 text-xs"
              >
                <HistoryEventIcon eventType={eventType} size={12} />
                {t(filterI18nKeys[eventType] ?? eventType)}
              </Button>
            );
          })}
        </div>

        <div className="flex-1 overflow-y-auto min-h-0">
          {loading && events.length === 0 ? (
            <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>{t("label.loading")}</span>
            </div>
          ) : (
            <>
              <HistoryEventTable events={events} />
              {hasMore ? (
                <div className="mt-4 flex justify-center pb-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={loading}
                    onClick={loadMore}
                  >
                    {loading ? (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    ) : null}
                    {t("history.loadMore")}
                  </Button>
                </div>
              ) : events.length > 0 ? (
                <p className="mt-4 pb-2 text-center text-xs text-muted-foreground">
                  {t("history.noMore")}
                </p>
              ) : null}
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

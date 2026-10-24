import * as React from "react";
import { AlertTriangle, X } from "lucide-react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { titleHistoryQuery } from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type { TitleHistoryEvent, TitleHistoryPage } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  HISTORY_TABLE_SHELL_CLASS,
  HistoryEventTable,
} from "./history-event-table";
import { TITLE_HISTORY_FILTERS } from "./title-history-event-meta";
import { LoadingMark } from "@/components/common/loading-mark";
import { HistoryEventTypeMultiSelect } from "@/components/common/history-event-type-multi-select";

const PAGE_SIZE = 50;

type ScopedEpisodeHistoryFilter = {
  episodeId: string;
  episodeLabel: string;
};

export function TitleHistoryModal({
  open,
  onOpenChange,
  titleId,
  titleName,
  scopedEpisode,
  scopedCollection,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId: string;
  titleName: string;
  scopedEpisode?: ScopedEpisodeHistoryFilter | null;
  scopedCollection?: { collectionId: string; collectionLabel: string } | null;
}) {
  const requestSequence = React.useRef(0);
  const client = useClient();
  const t = useTranslate();
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [totalCount, setTotalCount] = React.useState(0);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [activeFilters, setActiveFilters] = React.useState<string[]>([]);
  const [offset, setOffset] = React.useState(0);
  const [scopedEpisodeDismissed, setScopedEpisodeDismissed] =
    React.useState(false);
  const activeScopedEpisode = scopedEpisodeDismissed
    ? null
    : (scopedEpisode ?? null);

  const activeCollection = scopedEpisodeDismissed ? null : scopedCollection;

  const fetchHistory = React.useCallback(
    async (
      eventTypes: string[],
      pageOffset: number,
      append: boolean,
      episodeId?: string | null,
    ) => {
      const sequence = ++requestSequence.current;
      setLoading(true);
      setError(null);
      try {
        const result = await client
          .query<{ titleHistory: TitleHistoryPage }>(titleHistoryQuery, {
            filter: {
              eventTypes:
                eventTypes.length > 0
                  ? eventTypes.map((value) => value.toUpperCase())
                  : null,
              titleIds: [titleId],
              episodeId: episodeId ?? null,
              collectionId: activeCollection?.collectionId ?? null,
              groupByEvent: episodeId == null,
              limit: PAGE_SIZE,
              offset: pageOffset,
            },
          }, { requestPolicy: "network-only" })
          .toPromise();

        if (sequence !== requestSequence.current) return;
        if (result.error) {
          setError(
            userFacingGraphQlErrorMessage(
              result.error,
              t("history.loadError"),
            ),
          );
          return;
        }
        if (result.data?.titleHistory) {
          const page = result.data.titleHistory;
          setEvents((prev) =>
            append ? [...prev, ...page.items] : page.items,
          );
          setTotalCount(page.totalCount);
        } else {
          setError(t("history.loadError"));
        }
      } catch (requestError) {
        if (sequence !== requestSequence.current) return;
        setError(
          userFacingGraphQlErrorMessage(
            requestError,
            t("history.loadError"),
          ),
        );
      } finally {
        if (sequence === requestSequence.current) setLoading(false);
      }
    },
    [client, t, titleId, activeCollection?.collectionId],
  );

  React.useEffect(() => {
    setScopedEpisodeDismissed(false);
  }, [open, scopedEpisode?.episodeId, scopedCollection?.collectionId]);

  React.useEffect(() => {
    if (open) {
      setOffset(0);
      setEvents([]);
      void fetchHistory(
        activeFilters,
        0,
        false,
        activeScopedEpisode?.episodeId ?? null,
      );
    }
    return () => { requestSequence.current += 1; };
  }, [open, activeFilters, activeScopedEpisode?.episodeId, fetchHistory]);

  const loadMore = React.useCallback(() => {
    const nextOffset = offset + PAGE_SIZE;
    setOffset(nextOffset);
    void fetchHistory(
      activeFilters,
      nextOffset,
      true,
      activeScopedEpisode?.episodeId ?? null,
    );
  }, [offset, activeFilters, activeScopedEpisode?.episodeId, fetchHistory]);

  const retryHistory = React.useCallback(() => {
    setOffset(0);
    void fetchHistory(
      activeFilters,
      0,
      false,
      activeScopedEpisode?.episodeId ?? null,
    );
  }, [activeFilters, activeScopedEpisode?.episodeId, fetchHistory]);


  const clearScopedEpisode = React.useCallback(() => {
    setOffset(0);
    setEvents([]);
    setScopedEpisodeDismissed(true);
  }, []);

  const hasMore = events.length < totalCount;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="title-history-dialog"
        className="w-[calc(100%-1rem)] min-w-0 max-w-[95vw] sm:max-w-5xl lg:max-w-6xl max-h-[90dvh] overflow-hidden flex flex-col p-4 sm:p-6"
      >
        <DialogHeader className="min-w-0 gap-3 text-left lg:flex-row lg:items-center lg:justify-between lg:pr-8">
          <DialogTitle className="min-w-0 break-words pr-8 lg:pr-0">{titleName} — {t("history.title")}</DialogTitle>
          <HistoryEventTypeMultiSelect
            id="title-history-event-types"
            eventTypes={TITLE_HISTORY_FILTERS}
            selectedEventTypes={activeFilters}
            onSelectedEventTypesChange={setActiveFilters}
            triggerClassName="h-9 w-full min-w-0 lg:w-[320px] lg:shrink-0"
          />
        </DialogHeader>

        {activeScopedEpisode || activeCollection ? (
          <div className="flex flex-wrap items-center gap-2 pb-2">
            <span className="text-xs text-muted-foreground">{t("label.filter")}</span>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={clearScopedEpisode}
              aria-label={`${t("label.remove")} ${(activeScopedEpisode?.episodeLabel ?? activeCollection?.collectionLabel)}`}
              className="h-auto min-h-7 min-w-0 max-w-full gap-1.5 px-2 py-1 text-xs"
            >
              <span className="min-w-0 max-w-[32rem] whitespace-normal break-words text-left">
                {(activeScopedEpisode?.episodeLabel ?? activeCollection?.collectionLabel)}
              </span>
              <X className="h-3 w-3 shrink-0" />
            </Button>
          </div>
        ) : null}

        <div className="min-h-0 min-w-0 flex-1 overflow-y-auto">
          {error ? (
            <div
              role="alert"
              className="mb-3 flex flex-wrap items-center gap-3 rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]"
            >
              <AlertTriangle className="h-4 w-4 shrink-0" />
              <span className="min-w-0 flex-1">{error}</span>
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={loading}
                onClick={retryHistory}
              >
                {t("history.retry")}
              </Button>
            </div>
          ) : null}
          {loading && events.length === 0 ? (
            <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
              <LoadingMark className="h-4 w-4" />
              <span>{t("label.loading")}</span>
            </div>
          ) : error && events.length === 0 ? null : (
            <>
              <div className={cn(HISTORY_TABLE_SHELL_CLASS, "overflow-hidden")}>
                <HistoryEventTable
                  events={events}
                  showActor
                  emptyMessage={t("history.empty")}
                />
              </div>
              {!error && hasMore ? (
                <div className="mt-4 flex justify-center pb-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={loading}
                    onClick={loadMore}
                  >
                    {loading ? (
                      <LoadingMark className="mr-2 h-4 w-4" />
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

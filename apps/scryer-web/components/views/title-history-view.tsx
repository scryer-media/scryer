import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { TitleHistoryEvent } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { HistoryEventTable } from "@/components/common/history-event-table";
import {
  HistoryEventIcon,
  getEventTypeLabel,
} from "@/components/common/history-event-icon";

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

export function TitleHistoryView({
  events,
  totalCount,
  loading,
  activeFilters,
  availableFilters,
  onToggleFilter,
  onClearFilters,
  onLoadMore,
  hasMore,
}: {
  events: TitleHistoryEvent[];
  totalCount: number;
  loading: boolean;
  activeFilters: string[];
  availableFilters: string[];
  onToggleFilter: (eventType: string) => void;
  onClearFilters: () => void;
  onLoadMore: () => void;
  hasMore: boolean;
}) {
  const t = useTranslate();

  return (
    <Card>
      <CardHeader className="space-y-3">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <CardTitle>{t("history.title")}</CardTitle>
          <span className="text-sm text-muted-foreground">
            {totalCount} {totalCount === 1 ? "event" : "events"}
          </span>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            variant={activeFilters.length === 0 ? "default" : "secondary"}
            onClick={onClearFilters}
            className="text-xs"
          >
            {t("history.allEvents")}
          </Button>
          {availableFilters.map((eventType) => {
            const isActive = activeFilters.includes(eventType);
            return (
              <Button
                key={eventType}
                type="button"
                size="sm"
                variant={isActive ? "default" : "secondary"}
                onClick={() => onToggleFilter(eventType)}
                className="gap-1.5 text-xs"
              >
                <HistoryEventIcon eventType={eventType} size={14} />
                {t(filterI18nKeys[eventType] ?? eventType)}
              </Button>
            );
          })}
        </div>
      </CardHeader>
      <CardContent>
        {loading && events.length === 0 ? (
          <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : (
          <>
            <HistoryEventTable events={events} />
            {hasMore ? (
              <div className="mt-4 flex justify-center">
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={loading}
                  onClick={onLoadMore}
                >
                  {loading ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : null}
                  {t("history.loadMore")}
                </Button>
              </div>
            ) : events.length > 0 ? (
              <p className="mt-4 text-center text-xs text-muted-foreground">
                {t("history.noMore")}
              </p>
            ) : null}
          </>
        )}
      </CardContent>
    </Card>
  );
}

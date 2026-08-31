import { Loader2 } from "lucide-react";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import { HistoryEventTable } from "@/components/common/history-event-table";
import type { LibraryRecord, TitleHistoryEvent, TitleRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { HistoryEventIcon } from "@/components/common/history-event-icon";
import { getTitleHistoryFilterLabel } from "@/components/common/title-history-event-meta";

export function TitleHistoryView({
  events,
  totalCount,
  loading,
  error,
  activeFilters,
  availableFilters,
  selectedTitle,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  currentPage,
  pageSize,
  onSelectedTitleChange,
  onSelectedLibraryIdsChange,
  onToggleFilter,
  onClearFilters,
  onPreviousPage,
  onNextPage,
  onRetry,
  hasPreviousPage,
  hasNextPage,
}: {
  events: TitleHistoryEvent[];
  totalCount: number;
  loading: boolean;
  error: string | null;
  activeFilters: string[];
  availableFilters: string[];
  selectedTitle: TitleRecord | null;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  currentPage: number;
  pageSize: number;
  onSelectedTitleChange: (title: TitleRecord | null) => void;
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  onToggleFilter: (eventType: string) => void;
  onClearFilters: () => void;
  onPreviousPage: () => void;
  onNextPage: () => void;
  onRetry?: (importId: string, password?: string) => Promise<void>;
  hasPreviousPage: boolean;
  hasNextPage: boolean;
}) {
  const t = useTranslate();
  const pageStart = totalCount === 0 ? 0 : currentPage * pageSize + 1;
  const pageEnd = totalCount === 0 ? 0 : currentPage * pageSize + events.length;

  return (
    <Card className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-none border-0 bg-transparent shadow-none">
      <CardContent className="flex min-h-0 flex-1 flex-col gap-3 bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5">
        <div className="flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <TitleAutocompletePicker
              className="w-full sm:max-w-sm"
              placeholder={t("title.filterPlaceholder")}
              selectedTitle={selectedTitle}
              selectedTitleId={selectedTitle?.id ?? null}
              onSelectedTitleChange={onSelectedTitleChange}
            />
            <LibraryMultiSelect
              libraries={libraries}
              selectedLibraryIds={selectedLibraryIds}
              onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
              disabled={librariesLoading}
              triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[200px]"
            />
          </div>
          <div className="relative top-px flex min-h-10 min-w-0 max-w-full flex-wrap items-center justify-start gap-x-5 gap-y-1 border-0 bg-transparent p-0 shadow-none">
            <UnderlineFilterButton
              selected={activeFilters.length === 0}
              onClick={onClearFilters}
              label={t("history.allEvents")}
            />
            {availableFilters.map((eventType) => {
              const isActive = activeFilters.includes(eventType);
              return (
                <UnderlineFilterButton
                  key={eventType}
                  selected={isActive}
                  onClick={() => onToggleFilter(eventType)}
                  icon={<HistoryEventIcon eventType={eventType} size={14} />}
                  label={getTitleHistoryFilterLabel(eventType, t)}
                />
              );
            })}
          </div>
        </div>
        {loading && events.length === 0 ? (
          <div className="flex items-center gap-2 py-8 text-sm text-[var(--scry-muted3)]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : error ? (
          <p className="py-8 text-sm text-[var(--scry-danger-text)]">{error}</p>
        ) : (
          <div className="min-h-0 flex-1 overflow-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]">
            <HistoryEventTable
              events={events}
              showTitle
              showActor
              onRetry={onRetry}
              emptyMessage={t("history.empty")}
            />
          </div>
        )}
        <div className="flex shrink-0 flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[12.5px] text-[var(--scry-muted3)]">
            {t("pendingImports.pageRange", {
              start: pageStart,
              end: pageEnd,
              total: totalCount,
            })}
          </p>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-9 rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)]"
              disabled={!hasPreviousPage || loading}
              onClick={onPreviousPage}
            >
              {t("pendingImports.prev")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-9 rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)]"
              disabled={!hasNextPage || loading}
              onClick={onNextPage}
            >
              {t("pendingImports.next")}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Loader2, Search, Trash2, Zap } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { ViewId } from "@/components/root/types";
import type { Release, TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

function bytesToReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  if (raw > 1024 * 1024 * 1024) {
    return `${(raw / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  if (raw > 1024 * 1024) {
    return `${(raw / (1024 * 1024)).toFixed(2)} MB`;
  }
  if (raw > 1024) {
    return `${(raw / 1024).toFixed(2)} KB`;
  }
  return `${raw} B`;
}

type TitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  isDeletingById: Record<string, boolean>;
};

function resolveTitleProfileName(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
): string | null {
  const tag = item.tags?.find((t) => t.startsWith(QP_TAG_PREFIX));
  if (tag) {
    const id = tag.slice(QP_TAG_PREFIX.length);
    const match = profiles.find((p) => p.id === id);
    if (match) return match.name;
  }
  return fallback;
}

export function TitleTable({
  view,
  titles,
  titleLoading,
  resolvedProfileName,
  qualityProfiles,
  onOpenOverview,
  onDelete,
  onAutoQueue,
  onInteractiveSearch,
  onQueueFromInteractive,
  isDeletingById,
}: TitleTableProps) {
  const t = useTranslate();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = isMovieView ? "movies" : view === "anime" ? "anime" : "series";
  const columnCount = isMovieView ? 6 : 5;
  const titleTableColGroup = (
    <colgroup>
      <col style={{ width: "5.5rem" }} />
      <col />
      <col style={{ width: "10rem" }} />
      {isMovieView ? <col style={{ width: "8rem" }} /> : null}
      <col style={{ width: "7rem" }} />
      <col style={{ width: isMovieView ? "11rem" : "6rem" }} />
    </colgroup>
  );

  const [expandedMovieRows, setExpandedMovieRows] = React.useState(new Set<string>());
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] = React.useState<
    Record<string, Release[]>
  >({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<Record<string, boolean>>({});

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const useVirtualTable = titles.length > 50;
  const titleVirtualizer = useVirtualizer({
    count: titles.length,
    getScrollElement: () => titleTableScrollRef.current,
    estimateSize: () => 64,
    overscan: 5,
    measureElement: useVirtualTable ? (element) => element.getBoundingClientRect().height : undefined,
    enabled: useVirtualTable,
  });

  const handleQueueExisting = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setAutoQueueLoadingByTitle((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingByTitle((prev) => {
          if (!prev[titleId]) return prev;
          const next = { ...prev };
          delete next[titleId];
          return next;
        });
      });
    },
    [onAutoQueue],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onInteractiveSearch(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((prev) => ({
            ...prev,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((prev) => {
            if (!prev[titleId]) return prev;
            const next = { ...prev };
            delete next[titleId];
            return next;
          });
        });
    },
    [onInteractiveSearch],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedMovieRows.has(titleId);
      setExpandedMovieRows((prev) => {
        const next = new Set(prev);
        if (next.has(titleId)) {
          next.delete(titleId);
        } else {
          next.add(titleId);
        }
        return next;
      });
      if (!isOpen && !Object.prototype.hasOwnProperty.call(interactiveSearchResultsByTitle, titleId)) {
        handleRunInteractiveSearch(title);
      }
    },
    [expandedMovieRows, handleRunInteractiveSearch, interactiveSearchResultsByTitle],
  );

  const renderTitleRow = (item: TitleRecord) => {
    const isPanelOpen = isMovieView && expandedMovieRows.has(item.id);
    const interactiveSearchResults = interactiveSearchResultsByTitle[item.id] ?? [];
    const interactiveSearchLoading = interactiveSearchLoadingByTitle[item.id] === true;
    const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
    const deleteLoading = isDeletingById[item.id] === true;

    return (
      <React.Fragment key={item.id}>
        <TableRow className="h-24 cv-auto-row">
          <TableCell className="align-middle">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item.id)}
              className="inline-block text-left"
              aria-label={t("media.posterAlt", { name: item.name })}
            >
              <div className="h-20 w-14 overflow-hidden rounded border border-border bg-muted">
                {item.posterUrl ? (
                  <img
                    src={item.posterUrl}
                    alt={t("media.posterAlt", { name: item.name })}
                    className="h-full w-full object-cover"
                    loading="lazy"
                  />
                ) : (
                  <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground">
                    {t("label.noArt")}
                  </div>
                )}
              </div>
            </button>
          </TableCell>
          <TableCell className="align-middle overflow-hidden">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item.id)}
              className="block w-full overflow-hidden text-left text-xl font-bold hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          <TableCell className="align-middle whitespace-nowrap">
            {isMovieView
              ? (item.qualityTier || t("label.unknown"))
              : (resolveTitleProfileName(item, qualityProfiles, resolvedProfileName) || t("label.unknown"))}
          </TableCell>
          {isMovieView ? <TableCell className="align-middle whitespace-nowrap">{bytesToReadable(item.sizeBytes)}</TableCell> : null}
          <TableCell className="align-middle whitespace-nowrap">{item.monitored ? t("label.yes") : t("label.no")}</TableCell>
          <TableCell className="text-right align-middle">
            <div className="inline-flex items-center justify-end gap-2">
              {isMovieView ? (
                <>
                  <HoverCard openDelay={3000} closeDelay={75}>
                    <HoverCardTrigger asChild>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={t("label.search")}
                        onClick={() => handleQueueExisting(item)}
                        disabled={autoQueueLoading}
                      >
                        {autoQueueLoading ? (
                          <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
                        ) : (
                          <Zap className="h-4 w-4" />
                        )}
                      </Button>
                    </HoverCardTrigger>
                    <HoverCardContent>
                      <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                        {t("help.autoSearchTooltip")}
                      </p>
                    </HoverCardContent>
                  </HoverCard>
                  <HoverCard openDelay={3000} closeDelay={75}>
                    <HoverCardTrigger asChild>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={t("label.interactiveSearch")}
                        onClick={() => handleToggleInteractiveSearch(item)}
                      >
                        <Search className="h-4 w-4" />
                      </Button>
                    </HoverCardTrigger>
                    <HoverCardContent>
                      <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                        {t("help.interactiveSearchTooltip")}
                      </p>
                    </HoverCardContent>
                  </HoverCard>
                </>
              ) : null}
              <Button
                variant="destructive"
                size="sm"
                type="button"
                aria-label={t("label.delete")}
                onClick={() => onDelete(item)}
                disabled={deleteLoading}
              >
                {deleteLoading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Trash2 className="h-4 w-4" />
                )}
              </Button>
            </div>
          </TableCell>
        </TableRow>
        {isPanelOpen ? (
          <TableRow>
            <TableCell colSpan={columnCount} className="border-t border-border bg-popover/40 p-0">
              <div className="px-4 py-3">
                <div className="mb-2 flex items-center justify-between gap-3">
                  <p className="text-sm text-card-foreground">
                    {t("nzb.searchResultsFor", { name: item.name })}
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => handleRunInteractiveSearch(item)}
                    disabled={interactiveSearchLoading}
                    aria-label={t("label.search")}
                  >
                    <Search className="h-4 w-4" />
                    <span className="ml-1">
                      {interactiveSearchLoading ? t("label.searching") : t("label.refresh")}
                    </span>
                  </Button>
                </div>
                {interactiveSearchLoading ? (
                  <div className="flex items-center gap-3 py-3">
                    <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                    <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                  </div>
                ) : interactiveSearchResults.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
                ) : (
                  <SearchResultBuckets
                    results={interactiveSearchResults}
                    onQueue={(release) => onQueueFromInteractive(item, release)}
                  />
                )}
              </div>
            </TableCell>
          </TableRow>
        ) : null}
      </React.Fragment>
    );
  };

  const titleTableHeader = (
    <TableHeader>
      <TableRow>
        <TableHead className="whitespace-nowrap">{t("title.table.poster")}</TableHead>
        <TableHead>{t("title.table.name")}</TableHead>
        <TableHead className="whitespace-nowrap">{t("title.table.qualityTier")}</TableHead>
        {isMovieView ? <TableHead className="whitespace-nowrap">{t("title.table.size")}</TableHead> : null}
        <TableHead className="whitespace-nowrap">{t("title.table.monitored")}</TableHead>
        <TableHead className="text-right whitespace-nowrap">{t("title.table.actions")}</TableHead>
      </TableRow>
    </TableHeader>
  );

  if (!useVirtualTable) {
    return (
      <Table className="table-fixed">
        {titleTableColGroup}
        {titleTableHeader}
        <TableBody>
          {titles.map(renderTitleRow)}
          {titles.length === 0 && !titleLoading ? (
            <TableRow>
              <TableCell colSpan={columnCount} className="text-muted-foreground">
                {t("title.noManaged")}
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    );
  }

  const virtualItems = titleVirtualizer.getVirtualItems();

  return (
    <div
      ref={titleTableScrollRef}
      className="relative w-full"
      style={{ maxHeight: "70vh", overflow: "auto" }}
    >
      <table className="w-full table-fixed caption-bottom text-sm">
        {titleTableColGroup}
        <thead className="[&_tr]:border-b sticky top-0 z-10 bg-background">
          <TableRow>
            <TableHead className="whitespace-nowrap">{t("title.table.poster")}</TableHead>
            <TableHead>{t("title.table.name")}</TableHead>
            <TableHead className="whitespace-nowrap">{t("title.table.qualityTier")}</TableHead>
            {isMovieView ? <TableHead className="whitespace-nowrap">{t("title.table.size")}</TableHead> : null}
            <TableHead className="whitespace-nowrap">{t("title.table.monitored")}</TableHead>
            <TableHead className="text-right whitespace-nowrap">{t("title.table.actions")}</TableHead>
          </TableRow>
        </thead>
        {virtualItems.length > 0 ? (
          <>
            {virtualItems[0].start > 0 ? (
              <tbody aria-hidden>
                <tr><td style={{ height: virtualItems[0].start, padding: 0 }} /></tr>
              </tbody>
            ) : null}
            {virtualItems.map((virtualRow) => {
              const item = titles[virtualRow.index];
              return (
                <tbody
                  key={virtualRow.key}
                  ref={titleVirtualizer.measureElement}
                  data-index={virtualRow.index}
                  className="[&_tr:last-child]:border-0"
                >
                  {renderTitleRow(item)}
                </tbody>
              );
            })}
            {virtualItems[virtualItems.length - 1].end < titleVirtualizer.getTotalSize() ? (
              <tbody aria-hidden>
                <tr>
                  <td
                    style={{
                      height: titleVirtualizer.getTotalSize() - virtualItems[virtualItems.length - 1].end,
                      padding: 0,
                    }}
                  />
                </tr>
              </tbody>
            ) : null}
          </>
        ) : !titleLoading ? (
          <TableBody>
            <TableRow>
              <TableCell colSpan={columnCount} className="text-muted-foreground">
                {t("title.noManaged")}
              </TableCell>
            </TableRow>
          </TableBody>
        ) : null}
      </table>
    </div>
  );
}

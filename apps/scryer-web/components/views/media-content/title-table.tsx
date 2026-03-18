import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Eye, EyeOff, Loader2, Search, Trash2, Zap } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { ViewId } from "@/components/root/types";
import type { Release, TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

function formatProfileLabel(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.toLowerCase() === "4k") {
    return "4K";
  }
  if (/^\d{3,4}p$/i.test(trimmed)) {
    return trimmed.toUpperCase();
  }
  return trimmed;
}

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
  onToggleMonitored?: (title: TitleRecord, monitored: boolean) => Promise<void> | void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  isDeletingById: Record<string, boolean>;
  isTogglingMonitoredById?: Record<string, boolean>;
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
    return formatProfileLabel(id);
  }
  return formatProfileLabel(fallback) ?? fallback;
}

function resolveDisplayedQualityLabel(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
  unknownLabel: string,
) {
  return resolveTitleProfileName(item, profiles, fallback) || unknownLabel;
}

function StatusBadge({ status, t }: { status?: string | null; t: (key: string) => string }) {
  const normalized = status?.toLowerCase() ?? "";
  if (normalized === "ended") {
    return <span className="rounded bg-zinc-700/60 px-2 py-0.5 text-xs text-zinc-300">{t("title.ended")}</span>;
  }
  if (normalized === "upcoming") {
    return <span className="rounded bg-blue-900/50 px-2 py-0.5 text-xs text-blue-300">{t("title.upcoming")}</span>;
  }
  if (normalized === "continuing") {
    return <span className="rounded bg-emerald-900/50 px-2 py-0.5 text-xs text-emerald-300">{t("title.continuing")}</span>;
  }
  return null;
}

function TitleTableActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: BoxedActionButtonTone;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
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
  onToggleMonitored,
  onInteractiveSearch,
  onQueueFromInteractive,
  isDeletingById,
  isTogglingMonitoredById,
}: TitleTableProps) {
  "use no memo";
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
      <col style={{ width: "12.5rem" }} />
    </colgroup>
  );

  const [expandedInteractiveRows, setExpandedInteractiveRows] = React.useState(new Set<string>());
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] = React.useState<
    Record<string, Release[]>
  >({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<Record<string, boolean>>({});

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const titleVirtualizer = useVirtualizer({
    count: titles.length,
    getScrollElement: () => titleTableScrollRef.current,
    estimateSize: () => 96,
    overscan: 5,
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
      const isOpen = expandedInteractiveRows.has(titleId);
      setExpandedInteractiveRows((prev) => {
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
    [expandedInteractiveRows, handleRunInteractiveSearch, interactiveSearchResultsByTitle],
  );

  const renderTitleRow = (item: TitleRecord) => {
    const isPanelOpen = expandedInteractiveRows.has(item.id);
    const interactiveSearchResults = interactiveSearchResultsByTitle[item.id] ?? [];
    const interactiveSearchLoading = interactiveSearchLoadingByTitle[item.id] === true;
    const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
    const deleteLoading = isDeletingById[item.id] === true;
    const monitorToggleLoading = isTogglingMonitoredById?.[item.id] === true;
    const posterThumbUrl = selectPosterVariantUrl(item.posterUrl, "w70");

    return (
      <React.Fragment key={item.id}>
        <TableRow data-ui="title-table-row" className="h-24">
          <TableCell className="align-middle">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item.id)}
              data-ui="poster-link"
              className="inline-block text-left"
              aria-label={t("media.posterAlt", { name: item.name })}
            >
              <div data-ui="poster-thumb" className="h-20 w-14 overflow-hidden rounded border border-border bg-muted">
                {posterThumbUrl ? (
                  <img
                    src={posterThumbUrl}
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
              data-ui="title-name"
              className="block w-full overflow-hidden text-left text-xl font-bold hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          <TableCell className="text-center align-middle">
            <span
              className="inline-flex h-6 w-6 shrink-0 items-center justify-center"
              title={`${t("title.table.monitored")}: ${item.name}`}
              aria-label={`${t("title.table.monitored")}: ${item.name}`}
            >
              {item.monitored ? (
                <Eye className="h-5 w-5 text-emerald-600 dark:text-emerald-300" />
              ) : (
                <EyeOff className="h-5 w-5 text-rose-600 dark:text-rose-300" />
              )}
            </span>
          </TableCell>
          <TableCell className="align-middle whitespace-nowrap">
            {resolveDisplayedQualityLabel(
              item,
              qualityProfiles,
              resolvedProfileName,
              t("label.unknown"),
            )}
          </TableCell>
          {!isMovieView ? (
            <TableCell className="align-middle whitespace-nowrap">
              <StatusBadge status={item.contentStatus} t={t} />
            </TableCell>
          ) : null}
          {isMovieView ? <TableCell className="align-middle whitespace-nowrap">{bytesToReadable(item.sizeBytes)}</TableCell> : null}
          <TableCell className="text-center align-middle">
            <div data-ui="row-actions" className="inline-flex items-center justify-end gap-2">
              <HoverCard openDelay={3000} closeDelay={75}>
                <HoverCardTrigger asChild>
                  <TitleTableActionButton
                    tone="auto"
                    label={t("label.search")}
                    onClick={() => handleQueueExisting(item)}
                    disabled={autoQueueLoading}
                  >
                    {autoQueueLoading ? (
                      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
                    ) : (
                      <Zap className="h-4 w-4" />
                    )}
                  </TitleTableActionButton>
                </HoverCardTrigger>
                <HoverCardContent>
                  <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                    {t("help.autoSearchTooltip")}
                  </p>
                </HoverCardContent>
              </HoverCard>
              <HoverCard openDelay={3000} closeDelay={75}>
                <HoverCardTrigger asChild>
                  <TitleTableActionButton
                    tone="search"
                    label={t("label.interactiveSearch")}
                    onClick={() => handleToggleInteractiveSearch(item)}
                  >
                    <Search className="h-4 w-4" />
                  </TitleTableActionButton>
                </HoverCardTrigger>
                <HoverCardContent>
                  <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                    {t("help.interactiveSearchTooltip")}
                  </p>
                </HoverCardContent>
              </HoverCard>
              {onToggleMonitored ? (
                <TitleTableActionButton
                  tone={item.monitored ? "disabled" : "enabled"}
                  label={t(item.monitored ? "title.unmonitorAction" : "title.monitorAction")}
                  onClick={() => onToggleMonitored(item, !item.monitored)}
                  disabled={monitorToggleLoading}
                >
                  {monitorToggleLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : item.monitored ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </TitleTableActionButton>
              ) : null}
              <TitleTableActionButton
                tone="delete"
                label={t("label.delete")}
                onClick={() => onDelete(item)}
                disabled={deleteLoading}
              >
                {deleteLoading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Trash2 className="h-4 w-4" />
                )}
              </TitleTableActionButton>
            </div>
          </TableCell>
        </TableRow>
        {isPanelOpen ? (
          <TableRow data-ui="title-table-panel-row">
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
      <TableRow className="sticky top-0 z-10 bg-background">
        <TableHead className="w-14" />
        <TableHead>{t("label.name")}</TableHead>
        <TableHead className="text-center whitespace-nowrap">{t("title.table.monitored")}</TableHead>
        <TableHead className="w-48 whitespace-nowrap">{t("title.table.qualityTier")}</TableHead>
        {!isMovieView ? <TableHead className="whitespace-nowrap">{t("title.table.status")}</TableHead> : null}
        {isMovieView ? <TableHead className="whitespace-nowrap">{t("title.table.size")}</TableHead> : null}
        <TableHead className="text-center whitespace-nowrap">{t("label.actions")}</TableHead>
      </TableRow>
    </TableHeader>
  );

  const virtualItems = titleVirtualizer.getVirtualItems();


  return (
    <div
      ref={titleTableScrollRef}
      className="relative w-full"
      style={{ maxHeight: "70vh", overflow: "auto" }}
    >
      <table data-ui="title-table" data-view={view} className="w-full table-fixed caption-bottom text-sm">
        {titleTableColGroup}
        {titleTableHeader}
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

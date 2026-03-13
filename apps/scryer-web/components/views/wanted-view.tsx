import { Fragment, lazy, Suspense } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  ChevronDown,
  ChevronRight,
  Download,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  X,
} from "lucide-react";
import { CutoffUnmetView } from "@/components/views/cutoff-unmet-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import type { WantedItem, ReleaseDecisionItem, PendingReleaseItem } from "@/lib/types";
import type { WantedTab } from "@/components/containers/wanted-container";
import { useIsMobile } from "@/lib/hooks/use-mobile";

const CalendarView = lazy(() =>
  import("@/components/views/calendar-view").then((m) => ({ default: m.CalendarView })),
);

type CutoffUnmetViewState = {
  items: CutoffUnmetItem[];
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  searchingId: string | null;
  bulkSearching: boolean;
  bulkProgress: { current: number; total: number } | null;
  triggerSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => void;
};

type WantedViewState = {
  items: WantedItem[];
  total: number;
  loading: boolean;
  statusFilter: string | undefined;
  setStatusFilter: (v: string | undefined) => void;
  mediaTypeFilter: string | undefined;
  setMediaTypeFilter: (v: string | undefined) => void;
  offset: number;
  setOffset: (v: number) => void;
  limit: number;
  refreshItems: () => Promise<void>;
  expandedItemId: string | null;
  decisions: ReleaseDecisionItem[];
  decisionsLoading: boolean;
  loadDecisions: (id: string) => Promise<void>;
  triggerSearch: (id: string) => Promise<void>;
  pauseItem: (id: string) => Promise<void>;
  resumeItem: (id: string) => Promise<void>;
  resetItem: (id: string) => Promise<void>;
};

const STATUS_OPTIONS = ["wanted", "grabbed", "completed", "paused"] as const;
const MEDIA_TYPE_OPTIONS = ["movie", "episode"] as const;

function statusBadge(status: string) {
  const colors: Record<string, string> = {
    wanted: "bg-blue-500/20 text-blue-400",
    grabbed: "bg-amber-500/20 text-amber-400",
    completed: "bg-green-500/20 text-green-400",
    paused: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[status] ?? "bg-muted text-muted-foreground"}`}
    >
      {status}
    </span>
  );
}

function phaseBadge(phase: string) {
  const colors: Record<string, string> = {
    primary: "bg-green-500/20 text-green-400",
    pre_release: "bg-purple-500/20 text-purple-400",
    pre_air: "bg-purple-500/20 text-purple-400",
    secondary: "bg-yellow-500/20 text-yellow-400",
    long_tail: "bg-muted text-muted-foreground",
    paused: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[phase] ?? "bg-muted text-muted-foreground"}`}
    >
      {phase}
    </span>
  );
}

function decisionBadge(code: string) {
  const colors: Record<string, string> = {
    accept_initial: "bg-green-500/20 text-green-400",
    accept_upgrade: "bg-green-500/20 text-green-400",
    reject_insufficient_delta: "bg-red-500/20 text-red-400",
    reject_cooldown: "bg-amber-500/20 text-amber-400",
    reject_not_allowed: "bg-red-500/20 text-red-400",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[code] ?? "bg-muted text-muted-foreground"}`}
    >
      {code}
    </span>
  );
}

function formatDate(iso: string | null) {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function formatBytes(bytes: number | null) {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

type CalendarEpisodeItem = {
  id: string;
  titleId: string;
  titleName: string;
  titleFacet: string;
  seasonNumber: string | null;
  episodeNumber: string | null;
  episodeTitle: string | null;
  airDate: string | null;
  monitored: boolean;
};

type CalendarViewState = {
  episodes: CalendarEpisodeItem[];
  loading: boolean;
  onDateRangeChange: (start: string, end: string) => void;
  onEpisodeClick?: (episode: CalendarEpisodeItem) => void;
};

type PendingViewState = {
  items: PendingReleaseItem[];
  loading: boolean;
  refreshItems: () => Promise<void>;
  forceGrab: (id: string) => Promise<void>;
  dismiss: (id: string) => Promise<void>;
};

type WantedViewProps = {
  tab: WantedTab;
  onTabChange: (tab: WantedTab) => void;
  wantedState: WantedViewState;
  cutoffState: CutoffUnmetViewState;
  calendarState: CalendarViewState;
  pendingState: PendingViewState;
};

const TOGGLE_ITEM_CLASS =
  "h-full min-w-28 rounded-none px-4 text-sm font-semibold sm:min-w-36 sm:px-6 sm:text-base first:rounded-l-xl last:rounded-r-xl data-[state=off]:bg-accent/80 data-[state=off]:text-foreground data-[state=off]:hover:bg-accent/80 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border-0 data-[state=on]:shadow-none";

export function WantedView({ tab, onTabChange, wantedState, cutoffState, calendarState, pendingState }: WantedViewProps) {
  const t = useTranslate();

  return (
    <div className="space-y-4">
      <div className="overflow-x-auto">
        <ToggleGroup
          type="single"
          value={tab}
          onValueChange={(v) => {
            if (v) onTabChange(v as WantedTab);
          }}
          size="lg"
          className="mx-auto h-14 min-w-max rounded-xl border-0 bg-card/80 divide-x divide-border/40"
        >
          <ToggleGroupItem value="wanted" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabWanted")}
          </ToggleGroupItem>
          <ToggleGroupItem value="cutoff" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabCutoff")}
          </ToggleGroupItem>
          <ToggleGroupItem value="pending" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabPending")}
          </ToggleGroupItem>
          <ToggleGroupItem value="calendar" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabCalendar")}
          </ToggleGroupItem>
        </ToggleGroup>
      </div>

      {tab === "calendar" ? (
        <Suspense
          fallback={
            <Card>
              <CardContent className="p-8 text-center text-muted-foreground">
                {t("label.loading")}
              </CardContent>
            </Card>
          }
        >
          <CalendarView
            episodes={calendarState.episodes}
            loading={calendarState.loading}
            onDateRangeChange={calendarState.onDateRangeChange}
            onEpisodeClick={calendarState.onEpisodeClick}
          />
        </Suspense>
      ) : tab === "cutoff" ? (
        <CutoffUnmetView state={cutoffState} />
      ) : tab === "pending" ? (
        <PendingReleasesCard state={pendingState} />
      ) : (
        <WantedItemsCard state={wantedState} />
      )}
    </div>
  );
}

function WantedItemsCard({ state }: { state: WantedViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    items,
    total,
    loading,
    statusFilter,
    setStatusFilter,
    mediaTypeFilter,
    setMediaTypeFilter,
    offset,
    setOffset,
    limit,
    refreshItems,
    expandedItemId,
    decisions,
    decisionsLoading,
    loadDecisions,
    triggerSearch,
    pauseItem,
    resumeItem,
    resetItem,
  } = state;

  const hasPrev = offset > 0;
  const hasNext = offset + limit < total;

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle>{t("wanted.title")}</CardTitle>
          <Button
            className="w-full sm:w-auto"
            size="sm"
            variant="secondary"
            onClick={() => void refreshItems()}
            disabled={loading}
          >
            <RefreshCw className="mr-1 h-3 w-3" />
            {loading ? t("wanted.refreshing") : t("label.refresh")}
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:flex-wrap">
          <Select
            value={statusFilter ?? "__all__"}
            onValueChange={(v) => {
              setStatusFilter(v === "__all__" ? undefined : v);
              setOffset(0);
            }}
          >
            <SelectTrigger className="w-full sm:w-[150px]">
              <SelectValue placeholder={t("wanted.filterStatus")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("wanted.allStatuses")}</SelectItem>
              {STATUS_OPTIONS.map((s) => (
                <SelectItem key={s} value={s}>
                  {s}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <Select
            value={mediaTypeFilter ?? "__all__"}
            onValueChange={(v) => {
              setMediaTypeFilter(v === "__all__" ? undefined : v);
              setOffset(0);
            }}
          >
            <SelectTrigger className="w-full sm:w-[150px]">
              <SelectValue placeholder={t("wanted.filterMediaType")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("wanted.allTypes")}</SelectItem>
              {MEDIA_TYPE_OPTIONS.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <span className="self-center text-sm text-muted-foreground sm:ml-auto">
            {t("wanted.totalCount", { count: total })}
          </span>
        </div>

        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-muted-foreground">{t("wanted.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => (
                <div key={item.id} className="rounded-xl border border-border bg-card/30 p-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <button
                        type="button"
                        className="block text-left"
                        onClick={() => void loadDecisions(item.id)}
                      >
                        <p className="break-words text-sm font-medium text-foreground">
                          {item.titleName ?? item.titleId.slice(0, 8)}
                        </p>
                      </button>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {statusBadge(item.status)}
                        {phaseBadge(item.searchPhase)}
                        <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {item.mediaType}
                        </span>
                      </div>
                    </div>
                    <button
                      type="button"
                      className="p-0.5 text-muted-foreground hover:text-foreground"
                      onClick={() => void loadDecisions(item.id)}
                    >
                      {expandedItemId === item.id ? (
                        <ChevronDown className="h-4 w-4" />
                      ) : (
                        <ChevronRight className="h-4 w-4" />
                      )}
                    </button>
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <div>
                      <span className="block">{t("wanted.colNextSearch")}</span>
                      <span className="text-foreground">{formatDate(item.nextSearchAt)}</span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colScore")}</span>
                      <span className="text-foreground">{item.currentScore ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colSearches")}</span>
                      <span className="text-foreground">{item.searchCount}</span>
                    </div>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    <Button size="sm" variant="secondary" className="flex-1" onClick={() => void triggerSearch(item.id)}>
                      <Search className="h-4 w-4" />
                      <span>{t("wanted.searchNow")}</span>
                    </Button>
                    {item.status === "paused" ? (
                      <Button size="sm" variant="secondary" className="flex-1" onClick={() => void resumeItem(item.id)}>
                        <Play className="h-4 w-4" />
                        <span>{t("wanted.resume")}</span>
                      </Button>
                    ) : (
                      <Button size="sm" variant="secondary" className="flex-1" onClick={() => void pauseItem(item.id)}>
                        <Pause className="h-4 w-4" />
                        <span>{t("wanted.pause")}</span>
                      </Button>
                    )}
                    <Button size="sm" variant="outline" className="w-full" onClick={() => void resetItem(item.id)}>
                      <RotateCcw className="h-4 w-4" />
                      <span>{t("wanted.reset")}</span>
                    </Button>
                  </div>
                  {expandedItemId === item.id ? (
                    <div className="mt-3 border-t border-border pt-3">
                      {decisionsLoading ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.loadingDecisions")}</p>
                      ) : decisions.length === 0 ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.noDecisions")}</p>
                      ) : (
                        <div className="space-y-2">
                          {decisions.map((d) => (
                            <div key={d.id} className="rounded-lg border border-border bg-background/40 p-3">
                              <p className="break-words text-xs font-medium text-foreground">{d.releaseTitle}</p>
                              <div className="mt-2 flex flex-wrap gap-2">
                                {decisionBadge(d.decisionCode)}
                                <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                  {t("wanted.decScore")}: {d.candidateScore}
                                </span>
                                <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                  {t("wanted.decDelta")}: {d.scoreDelta ?? "—"}
                                </span>
                              </div>
                              <div className="mt-2 flex flex-wrap gap-3 text-xs text-muted-foreground">
                                <span>{formatBytes(d.releaseSizeBytes)}</span>
                                <span>{formatDate(d.createdAt)}</span>
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          )
        ) : (
          <div className="overflow-x-auto">
            <Table className="min-w-[980px]">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8" />
                  <TableHead>{t("wanted.colTitle")}</TableHead>
                  <TableHead>{t("wanted.colType")}</TableHead>
                  <TableHead>{t("wanted.colStatus")}</TableHead>
                  <TableHead>{t("wanted.colPhase")}</TableHead>
                  <TableHead>{t("wanted.colNextSearch")}</TableHead>
                  <TableHead>{t("wanted.colScore")}</TableHead>
                  <TableHead>{t("wanted.colSearches")}</TableHead>
                  <TableHead>{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => (
                  <Fragment key={item.id}>
                    <TableRow className="group">
                      <TableCell>
                        <button
                          className="p-0.5 text-muted-foreground hover:text-foreground"
                          onClick={() => void loadDecisions(item.id)}
                        >
                          {expandedItemId === item.id ? (
                            <ChevronDown className="h-4 w-4" />
                          ) : (
                            <ChevronRight className="h-4 w-4" />
                          )}
                        </button>
                      </TableCell>
                      <TableCell className="max-w-[200px] truncate text-sm" title={item.titleName ?? item.titleId}>
                        {item.titleName ?? item.titleId.slice(0, 8)}
                      </TableCell>
                      <TableCell>{item.mediaType}</TableCell>
                      <TableCell>{statusBadge(item.status)}</TableCell>
                      <TableCell>{phaseBadge(item.searchPhase)}</TableCell>
                      <TableCell className="text-xs">
                        {formatDate(item.nextSearchAt)}
                      </TableCell>
                      <TableCell>{item.currentScore ?? "—"}</TableCell>
                      <TableCell>{item.searchCount}</TableCell>
                      <TableCell>
                        <div className="flex gap-1">
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-7 w-7"
                            title={t("wanted.searchNow")}
                            onClick={() => void triggerSearch(item.id)}
                          >
                            <Search className="h-3.5 w-3.5" />
                          </Button>
                          {item.status === "paused" ? (
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("wanted.resume")}
                              onClick={() => void resumeItem(item.id)}
                            >
                              <Play className="h-3.5 w-3.5" />
                            </Button>
                          ) : (
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("wanted.pause")}
                              onClick={() => void pauseItem(item.id)}
                            >
                              <Pause className="h-3.5 w-3.5" />
                            </Button>
                          )}
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-7 w-7"
                            title={t("wanted.reset")}
                            onClick={() => void resetItem(item.id)}
                          >
                            <RotateCcw className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                    {expandedItemId === item.id && (
                      <TableRow>
                        <TableCell colSpan={9} className="bg-muted/30 p-4">
                          {decisionsLoading ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.loadingDecisions")}
                            </p>
                          ) : decisions.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.noDecisions")}
                            </p>
                          ) : (
                            <Table className="min-w-[720px]">
                              <TableHeader>
                                <TableRow>
                                  <TableHead>{t("wanted.decRelease")}</TableHead>
                                  <TableHead>{t("wanted.decDecision")}</TableHead>
                                  <TableHead>{t("wanted.decScore")}</TableHead>
                                  <TableHead>{t("wanted.decDelta")}</TableHead>
                                  <TableHead>{t("wanted.decSize")}</TableHead>
                                  <TableHead>{t("wanted.decDate")}</TableHead>
                                </TableRow>
                              </TableHeader>
                              <TableBody>
                                {decisions.map((d) => (
                                  <TableRow key={d.id}>
                                    <TableCell
                                      className="max-w-[300px] truncate text-xs"
                                      title={d.releaseTitle}
                                    >
                                      {d.releaseTitle}
                                    </TableCell>
                                    <TableCell>
                                      {decisionBadge(d.decisionCode)}
                                    </TableCell>
                                    <TableCell>{d.candidateScore}</TableCell>
                                    <TableCell>{d.scoreDelta ?? "—"}</TableCell>
                                    <TableCell className="text-xs">
                                      {formatBytes(d.releaseSizeBytes)}
                                    </TableCell>
                                    <TableCell className="text-xs">
                                      {formatDate(d.createdAt)}
                                    </TableCell>
                                  </TableRow>
                                ))}
                              </TableBody>
                            </Table>
                          )}
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                ))}
                {items.length === 0 && !loading && (
                  <TableRow>
                    <TableCell colSpan={9} className="text-center text-muted-foreground">
                      {t("wanted.noItems")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}

        {total > limit && (
          <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasPrev}
              onClick={() => setOffset(Math.max(0, offset - limit))}
            >
              {t("wanted.prev")}
            </Button>
            <span className="text-sm text-muted-foreground">
              {offset + 1}–{Math.min(offset + limit, total)} / {total}
            </span>
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasNext}
              onClick={() => setOffset(offset + limit)}
            >
              {t("wanted.next")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function formatTimeRemaining(delayUntil: string): string {
  const target = new Date(delayUntil).getTime();
  const now = Date.now();
  const diff = target - now;
  if (diff <= 0) return "now";
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const minutes = Math.floor((diff % (1000 * 60 * 60)) / (1000 * 60));
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function PendingReleasesCard({ state }: { state: PendingViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const { items, loading, refreshItems, forceGrab, dismiss } = state;

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle>{t("pending.title")}</CardTitle>
          <Button
            className="w-full sm:w-auto"
            size="sm"
            variant="secondary"
            onClick={() => void refreshItems()}
            disabled={loading}
          >
            <RefreshCw className="mr-1 h-3 w-3" />
            {loading ? t("wanted.refreshing") : t("label.refresh")}
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-muted-foreground">{t("pending.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => (
                <div key={item.id} className="rounded-xl border border-border bg-card/30 p-3">
                  <p className="break-words text-sm font-medium text-foreground">{item.releaseTitle}</p>
                  <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <div>
                      <span className="block">{t("pending.colScore")}</span>
                      <span className="text-foreground">{item.releaseScore}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colSize")}</span>
                      <span className="text-foreground">{item.releaseSizeBytes ? formatBytes(Number(item.releaseSizeBytes)) : "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colIndexer")}</span>
                      <span className="text-foreground">{item.indexerSource ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colDelayUntil")}</span>
                      <span className="text-foreground" title={formatDate(item.delayUntil)}>
                        {formatTimeRemaining(item.delayUntil)}
                      </span>
                    </div>
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">{formatDate(item.addedAt)}</p>
                  <div className="mt-3 flex gap-2">
                    <Button size="sm" variant="secondary" className="flex-1" onClick={() => void forceGrab(item.id)}>
                      <Download className="h-4 w-4" />
                      <span>{t("pending.forceGrab")}</span>
                    </Button>
                    <Button size="sm" variant="outline" className="flex-1" onClick={() => void dismiss(item.id)}>
                      <X className="h-4 w-4" />
                      <span>{t("pending.dismiss")}</span>
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )
        ) : (
          <div className="overflow-x-auto">
            <Table className="min-w-[760px]">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("pending.colRelease")}</TableHead>
                  <TableHead>{t("pending.colScore")}</TableHead>
                  <TableHead>{t("pending.colSize")}</TableHead>
                  <TableHead>{t("pending.colIndexer")}</TableHead>
                  <TableHead>{t("pending.colAddedAt")}</TableHead>
                  <TableHead>{t("pending.colDelayUntil")}</TableHead>
                  <TableHead>{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => (
                  <TableRow key={item.id}>
                    <TableCell className="max-w-[300px] truncate text-sm" title={item.releaseTitle}>
                      {item.releaseTitle}
                    </TableCell>
                    <TableCell>{item.releaseScore}</TableCell>
                    <TableCell className="text-xs">
                      {item.releaseSizeBytes ? formatBytes(Number(item.releaseSizeBytes)) : "—"}
                    </TableCell>
                    <TableCell className="text-xs">{item.indexerSource ?? "—"}</TableCell>
                    <TableCell className="text-xs">{formatDate(item.addedAt)}</TableCell>
                    <TableCell className="text-xs">
                      <span title={formatDate(item.delayUntil)}>
                        {formatTimeRemaining(item.delayUntil)}
                      </span>
                    </TableCell>
                    <TableCell>
                      <div className="flex gap-1">
                        <Button
                          size="icon"
                          variant="ghost"
                          className="h-7 w-7"
                          title={t("pending.forceGrab")}
                          onClick={() => void forceGrab(item.id)}
                        >
                          <Download className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          size="icon"
                          variant="ghost"
                          className="h-7 w-7"
                          title={t("pending.dismiss")}
                          onClick={() => void dismiss(item.id)}
                        >
                          <X className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
                {items.length === 0 && !loading && (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center text-muted-foreground">
                      {t("pending.noItems")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

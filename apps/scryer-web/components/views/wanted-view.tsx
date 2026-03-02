import { lazy, Suspense } from "react";
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
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
} from "lucide-react";
import { CutoffUnmetView } from "@/components/views/cutoff-unmet-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import type { WantedItem, ReleaseDecisionItem } from "@/lib/types";
import type { WantedTab } from "@/components/containers/wanted-container";

const CalendarView = lazy(() =>
  import("@/components/views/calendar-view").then((m) => ({ default: m.CalendarView })),
);

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type CutoffUnmetViewState = {
  t: Translate;
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
  t: Translate;
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
  t: Translate;
  episodes: CalendarEpisodeItem[];
  loading: boolean;
  onDateRangeChange: (start: string, end: string) => void;
  onEpisodeClick?: (episode: CalendarEpisodeItem) => void;
};

type WantedViewProps = {
  tab: WantedTab;
  onTabChange: (tab: WantedTab) => void;
  wantedState: WantedViewState;
  cutoffState: CutoffUnmetViewState;
  calendarState: CalendarViewState;
};

const TOGGLE_ITEM_CLASS =
  "h-full min-w-36 rounded-none px-6 text-base font-semibold first:rounded-l-xl last:rounded-r-xl data-[state=off]:bg-accent/80 data-[state=off]:text-foreground data-[state=off]:hover:bg-accent/80 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border-0 data-[state=on]:shadow-none";

export function WantedView({ tab, onTabChange, wantedState, cutoffState, calendarState }: WantedViewProps) {
  const { t } = wantedState;

  return (
    <div className="space-y-4">
      <div className="flex justify-center">
        <ToggleGroup
          type="single"
          value={tab}
          onValueChange={(v) => {
            if (v) onTabChange(v as WantedTab);
          }}
          size="lg"
          className="h-14 rounded-xl border-0 bg-card/80 overflow-hidden divide-x divide-border/40"
        >
          <ToggleGroupItem value="wanted" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabWanted")}
          </ToggleGroupItem>
          <ToggleGroupItem value="cutoff" size="lg" className={TOGGLE_ITEM_CLASS}>
            {t("wanted.tabCutoff")}
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
            t={calendarState.t}
          />
        </Suspense>
      ) : tab === "cutoff" ? (
        <CutoffUnmetView state={cutoffState} />
      ) : (
        <WantedItemsCard state={wantedState} />
      )}
    </div>
  );
}

function WantedItemsCard({ state }: { state: WantedViewState }) {
  const {
    t,
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
        <div className="flex items-center justify-between">
          <CardTitle>{t("wanted.title")}</CardTitle>
          <Button
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
        <div className="mb-4 flex flex-wrap gap-3">
          <Select
            value={statusFilter ?? "__all__"}
            onValueChange={(v) => {
              setStatusFilter(v === "__all__" ? undefined : v);
              setOffset(0);
            }}
          >
            <SelectTrigger className="w-[150px]">
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
            <SelectTrigger className="w-[150px]">
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

          <span className="self-center text-sm text-muted-foreground">
            {t("wanted.totalCount", { count: total })}
          </span>
        </div>

        <div className="overflow-x-auto">
          <Table>
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
                <TableHead>{t("wanted.colActions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((item) => (
                <>
                  <TableRow key={item.id} className="group">
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
                    <TableRow key={`${item.id}-decisions`}>
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
                          <Table>
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
                </>
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

        {total > limit && (
          <div className="mt-4 flex items-center justify-between">
            <Button
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

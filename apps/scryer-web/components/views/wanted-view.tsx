import { Fragment, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import type { OverviewTitleTarget, ViewId, WantedSection } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Translate } from "@/components/root/types";
import { buildViewPath } from "@/lib/utils/routing";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId, wantedItemRowId, wantedItemSearchNowId } from "@/lib/utils/dom-ids";
import { parseDecisionExplanation } from "@/lib/utils/release-decision-explanation";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableCodeCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ChevronDown,
  ChevronRight,
  Clock,
  Download,
  Gauge,
  History,
  ListChecks,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  X,
} from "lucide-react";
import { ConvergenceBadge } from "@/components/views/convergence-badge";
import { CutoffUnmetView } from "@/components/views/cutoff-unmet-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import { WantedScoringBreakdown } from "@/components/views/wanted-scoring-breakdown";
import type {
  AcquisitionSearchJob,
  PendingReleaseItem,
  PendingReleaseStatus,
  LibraryRecord,
  Release,
  ReleaseDecisionItem,
  TitleRecord,
  WantedItem,
  WantedMediaType,
  WantedStatus,
} from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type CutoffUnmetViewState = {
  items: CutoffUnmetItem[];
  total: number;
  offset: number;
  setOffset: (v: number) => void;
  limit: number;
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  activeInteractiveItemId: string | null;
  searchResultsByItemId: Record<string, Release[]>;
  searchJob: AcquisitionSearchJob | null;
  searchJobStarting: boolean;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
  queueRelease: (item: CutoffUnmetItem, release: Release) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => Promise<void>;
};

type WantedViewState = {
  items: WantedItem[];
  total: number;
  loading: boolean;
  selectedTitle: TitleRecord | null;
  setSelectedTitle: (title: TitleRecord | null) => void;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
  offset: number;
  setOffset: (v: number) => void;
  limit: number;
  refreshItems: () => Promise<void>;
  expandedItemId: string | null;
  decisions: ReleaseDecisionItem[];
  decisionsLoading: boolean;
  standbyReleases: PendingReleaseItem[];
  standbyLoading: boolean;
  loadItemDetails: (id: string, standbyCount: number) => Promise<void>;
  triggerSearch: (id: string) => Promise<void>;
  pauseItem: (id: string) => Promise<void>;
  resumeItem: (id: string) => Promise<void>;
  triggerMismatchRecovery: (titleId: string) => Promise<void>;
};

function formatWantedMediaType(mediaType: WantedMediaType, t: Translate) {
  const key: Record<WantedMediaType, string> = {
    MOVIE: "wanted.type.movie",
    EPISODE: "wanted.type.episode",
    SERIES_MOVIE: "wanted.type.seriesMovie",
  };
  return t(key[mediaType]);
}

function formatWantedStatus(status: WantedStatus, t: Translate) {
  const key: Record<WantedStatus, string> = {
    WANTED: "wanted.status.wanted",
    GRABBED: "wanted.status.grabbed",
    COMPLETED: "wanted.status.completed",
    PAUSED: "wanted.status.paused",
  };
  return t(key[status]);
}

function formatWantedDecisionCode(code: string, t: Translate) {
  const key = {
    eligible: "wanted.decision.eligible",
    title_mismatch: "wanted.decision.titleMismatch",
    episode_mismatch: "wanted.decision.episodeMismatch",
    category_mismatch: "wanted.decision.categoryMismatch",
    ambiguous_identity: "wanted.decision.ambiguousIdentity",
    quality_blocked: "wanted.decision.qualityBlocked",
    minimum_seeders: "wanted.decision.minimumSeeders",
    pack_below_missing_threshold: "wanted.decision.packBelowMissingThreshold",
    upgrade_rejected: "wanted.decision.upgradeRejected",
    pending_delay: "wanted.decision.pendingDelay",
    already_active: "wanted.decision.alreadyActive",
    accept_initial: "wanted.decision.acceptInitial",
    accept_upgrade: "wanted.decision.acceptUpgrade",
    reject_insufficient_delta: "wanted.decision.rejectInsufficientDelta",
    reject_cooldown: "wanted.decision.rejectCooldown",
    reject_not_allowed: "wanted.decision.rejectNotAllowed",
  }[code];
  return key ? t(key) : code;
}

function wantedItemContext(item: WantedItem, t: Translate) {
  if (item.mediaType === "SERIES_MOVIE") {
    return t("wanted.context.seriesMovie");
  }
  if (item.mediaType === "EPISODE" && item.seasonNumber) {
    return t("wanted.context.seasonEpisode", {
      seasonNumber: item.seasonNumber,
    });
  }
  if (item.mediaType === "EPISODE") {
    return t("wanted.context.episode");
  }
  return t("wanted.context.movie");
}

function wantedItemOverviewView(item: WantedItem): ViewId | null {
  switch (item.titleFacet) {
    case "movie":
      return "movies";
    case "series":
      return "series";
    case "anime":
      return "anime";
    default:
      return null;
  }
}

function wantedItemOverviewTarget(item: WantedItem): OverviewTitleTarget | null {
  const normalizedTitleId = item.titleId.trim();
  if (!normalizedTitleId) {
    return null;
  }

  const normalizedSlug = item.titleSlug?.trim() || null;
  return {
    id: normalizedTitleId,
    slug: normalizedSlug,
    libraryId: item.libraryId,
    librarySlug: item.librarySlug,
  };
}

function formatWantedEpisodeCode(item: WantedItem): string | null {
  if (item.mediaType !== "EPISODE") {
    return null;
  }

  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }

  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function wantedItemSubtitle(item: WantedItem, t: Translate): string {
  return formatWantedEpisodeCode(item) ?? wantedItemContext(item, t);
}

function statusBadge(status: WantedStatus, t: Translate) {
  const colors: Record<WantedStatus, string> = {
    WANTED: "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]",
    GRABBED: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
    COMPLETED: "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]",
    PAUSED: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[status] ?? "bg-muted text-muted-foreground"}`}
    >
      {formatWantedStatus(status, t)}
    </span>
  );
}

function decisionBadge(code: string, t: Translate) {
  const colors: Record<string, string> = {
    eligible: "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]",
    title_mismatch: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    episode_mismatch: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    category_mismatch: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    ambiguous_identity: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    quality_blocked: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    upgrade_rejected: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
    pending_delay: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
    already_active: "bg-muted text-muted-foreground",
    download_client_unavailable: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
    repack_group_mismatch: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    accept_initial: "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]",
    accept_upgrade: "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]",
    reject_insufficient_delta: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
    reject_cooldown: "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]",
    reject_not_allowed: "bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text-soft)]",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[code] ?? "bg-muted text-muted-foreground"}`}
    >
      {formatWantedDecisionCode(code, t)}
    </span>
  );
}

function formatDate(iso: string | null, dateTimeFormat: UiDateTimeFormat) {
  return formatUiDateTime(iso, dateTimeFormat, { fallback: "—" });
}

function formatBytes(bytes: number | null) {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function isSeasonPackRelease(title: string) {
  return /(?:^|[ ._-])S\d{1,2}(?:$|[ ._-])/i.test(title) && !/S\d{1,2}E\d{1,3}/i.test(title);
}

function StandbyReleasesList({
  items,
  loading,
}: {
  items: PendingReleaseItem[];
  loading: boolean;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();

  if (loading) {
    return <p className="text-sm text-muted-foreground">{t("wanted.loadingStandby")}</p>;
  }
  if (items.length === 0) {
    return null;
  }

  return (
    <section className="space-y-2" data-ui="wanted-standby-list">
      <h4 className="text-sm font-medium text-foreground">
        {t("wanted.standbyCandidates", { count: items.length })}
      </h4>
      <div className="space-y-2">
        {items.map((release, index) => (
          <div
            key={release.id}
            className="rounded-lg border border-border bg-background/40 p-3"
          >
            <div className="flex flex-wrap items-start gap-2">
              <span className="rounded bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
                {t("wanted.standbyRank", { rank: index + 1 })}
              </span>
              {isSeasonPackRelease(release.releaseTitle) ? (
                <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                  {t("wanted.seasonPack")}
                </span>
              ) : null}
              <p className="min-w-0 flex-1 break-words text-xs font-medium text-foreground">
                {release.releaseTitle}
              </p>
            </div>
            <div className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1 text-xs text-muted-foreground sm:grid-cols-3">
              <span>{t("wanted.standbyIndexer")}: {release.indexerSource ?? "—"}</span>
              <span>{t("wanted.standbySize")}: {formatBytes(release.releaseSizeBytes)}</span>
              {release.seeders == null ? null : (
                <span>{t("wanted.standbySeeders")}: {release.seeders}</span>
              )}
              <span>{t("wanted.standbyScore")}: {release.releaseScore}</span>
              <span>{t("wanted.standbyAge")}: {formatDate(release.publishedAt ?? release.addedAt, dateTimeFormat)}</span>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function StandbyChip({
  item,
  expanded,
  onExpand,
}: {
  item: WantedItem;
  expanded: boolean;
  onExpand: () => void;
}) {
  const t = useTranslate();
  if (item.standbyCount <= 0) {
    return null;
  }
  const tooltip = `${t("wanted.standbyTooltip", { count: item.standbyCount })} ${t("wanted.standbyScopeNote")}`;
  return (
    <button
      type="button"
      className="rounded border border-[rgba(var(--scry-accent-rgb),0.35)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-2 py-0.5 text-xs font-medium text-[var(--scry-accent-text)] hover:bg-[rgba(var(--scry-accent-rgb),0.2)]"
      title={tooltip}
      aria-label={tooltip}
      aria-expanded={expanded}
      onClick={onExpand}
    >
      {t("wanted.standby", { count: item.standbyCount })}
    </button>
  );
}

function NoStandbyCandidates({ item }: { item: WantedItem }) {
  const t = useTranslate();
  if (
    item.status !== "WANTED" ||
    item.convergenceState !== "CONVERGED" ||
    item.standbyCount !== 0
  ) {
    return null;
  }
  return <span className="text-xs text-muted-foreground">{t("wanted.noStandbyCandidates")}</span>;
}

type PendingViewState = {
  items: PendingReleaseItem[];
  total: number;
  loading: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  refreshItems: () => Promise<void>;
  loadMoreItems: () => Promise<void>;
  forceGrab: (id: string) => Promise<void>;
  dismiss: (id: string) => Promise<void>;
};

type WantedViewProps = {
  section: WantedSection;
  wantedState: WantedViewState;
  cutoffState: CutoffUnmetViewState;
  pendingState: PendingViewState;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

export function WantedView({
  section,
  wantedState,
  cutoffState,
  pendingState,
  onOpenOverview,
}: WantedViewProps) {
  const t = useTranslate();
  const wantedNav: Array<{
    section: WantedSection | "history";
    label: string;
    count: number | null;
    icon: typeof ListChecks;
    to: string;
    active: boolean;
  }> = [
    {
      section: "wanted",
      label: t("wanted.title"),
      count: wantedState.total,
      icon: ListChecks,
      to: buildViewPath("wanted", undefined, undefined, undefined, "wanted"),
      active: section === "wanted",
    },
    {
      section: "cutoff",
      label: t("cutoff.title"),
      count: cutoffState.total,
      icon: Gauge,
      to: buildViewPath("wanted", undefined, undefined, undefined, "cutoff"),
      active: section === "cutoff",
    },
    {
      section: "pending",
      label: t("pending.title"),
      count: pendingState.total,
      icon: Clock,
      to: buildViewPath("wanted", undefined, undefined, undefined, "pending"),
      active: section === "pending",
    },
    {
      section: "history",
      label: t("history.title"),
      count: null,
      icon: History,
      to: buildViewPath("activity", undefined, undefined, undefined, undefined, "history"),
      active: false,
    },
  ];
  const activeWantedNavItem =
    wantedNav.find((item) => item.active) ?? wantedNav[0]!;
  const ActiveWantedIcon = activeWantedNavItem.icon;

  return (
    <div className="md:flex md:h-full md:min-h-0 md:flex-row">
      <aside className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]">
        <nav
          className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0"
          aria-label={t("nav.wanted")}
        >
          {wantedNav.map((item) => {
            const Icon = item.icon;
            const active = item.active;
            return (
              <Link
                key={item.section}
                to={item.to}
                className={cn(
                  "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                  active &&
                    "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                )}
              >
                <Icon
                  className={cn(
                    "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                    active && "text-[var(--scry-accent-text)]",
                  )}
                />
                <span className="whitespace-nowrap">{item.label}</span>
                {item.count === null ? null : (
                  <span
                    className={cn(
                      "ml-auto rounded-md bg-[var(--scry-chip)] px-1.5 py-0.5 text-[10.5px] font-semibold tabular-nums text-[var(--scry-muted3)]",
                      active && "bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]",
                    )}
                  >
                    {item.count}
                  </span>
                )}
              </Link>
            );
          })}
        </nav>
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto bg-transparent">
        <div className="mx-auto flex min-h-full w-full max-w-none flex-col px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]">
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{t("nav.group.automation")}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">
              {activeWantedNavItem.label}
            </span>
          </div>
          <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <ActiveWantedIcon className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {activeWantedNavItem.label}
                </h1>
              </div>
            </div>
          </div>
          <div className="min-h-0 flex-1">
            {section === "cutoff" ? (
              <CutoffUnmetView state={cutoffState} />
            ) : section === "pending" ? (
              <PendingReleasesCard state={pendingState} />
            ) : (
              <WantedItemsCard state={wantedState} onOpenOverview={onOpenOverview} />
            )}
          </div>
        </div>
      </main>
    </div>
  );
}

function WantedItemsCard({
  state,
  onOpenOverview,
}: {
  state: WantedViewState;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMobile = useIsMobile();
  const {
    items,
    total,
    loading,
    selectedTitle,
    setSelectedTitle,
    libraries,
    librariesLoading,
    selectedLibraryIds,
    setSelectedLibraryIds,
    offset,
    setOffset,
    limit,
    refreshItems,
    expandedItemId,
    decisions,
    decisionsLoading,
    standbyReleases,
    standbyLoading,
    loadItemDetails,
    triggerSearch,
    pauseItem,
    resumeItem,
    triggerMismatchRecovery,
  } = state;
  const [expandedDecisionIds, setExpandedDecisionIds] = useState<Set<string>>(new Set());

  const hasPrev = offset > 0;
  const hasNext = offset + limit < total;
  const shouldScrollDesktopTable = !isMobile;

  useEffect(() => {
    setExpandedDecisionIds(new Set());
  }, [expandedItemId, decisions]);

  const toggleDecisionScoring = (decisionId: string) => {
    setExpandedDecisionIds((current) => {
      const next = new Set(current);
      if (next.has(decisionId)) {
        next.delete(decisionId);
      } else {
        next.add(decisionId);
      }
      return next;
    });
  };

  const openWantedItemOverview = (item: WantedItem) => {
    if (!onOpenOverview) {
      return;
    }

    const targetView = wantedItemOverviewView(item);
    const overviewTarget = wantedItemOverviewTarget(item);
    if (!targetView || !overviewTarget) {
      return;
    }

    onOpenOverview(targetView, overviewTarget, item.episodeId ?? undefined);
  };

  return (
    <Card
      className={
        shouldScrollDesktopTable
          ? "flex min-h-0 flex-1 flex-col overflow-hidden rounded-none border-0 bg-transparent shadow-none"
          : "overflow-hidden rounded-none border-0 bg-transparent shadow-none"
      }
    >
      <CardHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-end">
          <Button
            className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)] sm:w-auto"
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
      <CardContent
        className={
          shouldScrollDesktopTable
            ? "flex min-h-0 flex-1 flex-col space-y-3 bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5"
            : "space-y-4 bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5"
        }
      >
        <div className="flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3 sm:flex-row sm:flex-wrap sm:items-center">
          <TitleAutocompletePicker
            ariaLabel={t("wanted.filterTitle")}
            className="w-full sm:max-w-sm"
            placeholder={t("wanted.filterTitlePlaceholder")}
            selectedTitle={selectedTitle}
            selectedTitleId={selectedTitle?.id ?? null}
            onSelectedTitleChange={setSelectedTitle}
          />

          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={(libraryIds) => {
              setSelectedLibraryIds(libraryIds);
              setOffset(0);
            }}
            disabled={librariesLoading || libraries.length === 0}
            triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[180px]"
          />

          <span className="self-center text-sm font-medium text-[var(--scry-muted3)] sm:ml-auto">
            {t("wanted.totalCount", { count: total })}
          </span>
        </div>

        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("wanted.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => (
                <div key={item.id} className="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <button
                        type="button"
                        className="block text-left hover:text-foreground"
                        onClick={() => openWantedItemOverview(item)}
                      >
                        <p className="break-words text-sm font-medium text-foreground hover:underline">
                          {item.titleName ?? item.titleId.slice(0, 8)}
                        </p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {wantedItemSubtitle(item, t)}
                        </p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {item.libraryName ?? item.libraryId ?? "Library"}
                        </p>
                        {item.sourceProvider ? (
                          <p className="mt-1 truncate text-xs text-muted-foreground">
                            {item.sourceProvider}
                          </p>
                        ) : null}
                      </button>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {statusBadge(item.status, t)}
                        <ConvergenceBadge
                          state={item.convergenceState}
                          indexersCovered={item.indexersCovered}
                          indexersRouted={item.indexersRouted}
                          recencyLane={item.recencyLane}
                        />
                        <NoStandbyCandidates item={item} />
                        <StandbyChip
                          item={item}
                          expanded={expandedItemId === item.id}
                          onExpand={() => void loadItemDetails(item.id, item.standbyCount)}
                        />
                        <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {formatWantedMediaType(item.mediaType, t)}
                        </span>
                      </div>
                    </div>
                    <button
                      type="button"
                      className="p-0.5 text-muted-foreground hover:text-foreground"
                      onClick={() => void loadItemDetails(item.id, item.standbyCount)}
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
                      <span className="block">{t("wanted.colLastSearch")}</span>
                      <span className="text-foreground">
                        {formatDate(item.lastSearchAt, dateTimeFormat)}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colLatestDecision")}</span>
                      <span className="text-foreground">
                        {item.latestReleaseDecision
                          ? formatWantedDecisionCode(
                              item.latestReleaseDecision.decisionCode,
                              t,
                            )
                          : "—"}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colScore")}</span>
                      <span className="text-foreground">{item.currentScore ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colIndexers")}</span>
                      <span className="text-foreground">
                        {item.indexersCovered}/{item.indexersRouted}
                      </span>
                    </div>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    <Button size="sm" variant="secondary" className="flex-1" onClick={() => void triggerSearch(item.id)}>
                      <Search className="h-4 w-4" />
                      <span>{t("wanted.searchNow")}</span>
                    </Button>
                    {item.status === "PAUSED" ? (
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
                    {item.mismatchRecoveryEligible ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="w-full"
                        onClick={() => void triggerMismatchRecovery(item.titleId)}
                      >
                        <RotateCcw className="h-4 w-4" />
                        <span>{t("wanted.actionRecoverMismatch")}</span>
                      </Button>
                    ) : null}
                  </div>
                  {expandedItemId === item.id ? (
                    <div className="mt-3 border-t border-border pt-3">
                      <StandbyReleasesList items={standbyReleases} loading={standbyLoading} />
                      {decisionsLoading ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.loadingDecisions")}</p>
                      ) : decisions.length === 0 ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.noDecisions")}</p>
                      ) : (
                        <div className="space-y-2">
                          {decisions.map((d) => {
                            const scoringEntries = parseDecisionExplanation(d.explanationJson);
                            const hasScoringBreakdown = scoringEntries.length > 0;
                            const scoringExpanded = expandedDecisionIds.has(d.id);

                            return (
                              <div key={d.id} className="rounded-lg border border-border bg-background/40 p-3">
                                <p className="break-words text-xs font-medium text-foreground">{d.releaseTitle}</p>
                                <div className="mt-2 flex flex-wrap gap-2">
                                  {decisionBadge(d.decisionCode, t)}
                                  <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                    {t("wanted.decScore")}: {d.candidateScore}
                                  </span>
                                  <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                    {t("wanted.decDelta")}: {d.scoreDelta ?? "—"}
                                  </span>
                                </div>
                                <div className="mt-2 flex flex-wrap gap-3 text-xs text-muted-foreground">
                                  <span>{formatBytes(d.releaseSizeBytes)}</span>
                                  <span>{formatDate(d.createdAt, dateTimeFormat)}</span>
                                </div>
                                {hasScoringBreakdown ? (
                                  <div className="mt-3 border-t border-border pt-3">
                                    <button
                                      type="button"
                                      className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
                                      onClick={() => toggleDecisionScoring(d.id)}
                                    >
                                      {scoringExpanded ? (
                                        <ChevronDown className="h-3.5 w-3.5" />
                                      ) : (
                                        <ChevronRight className="h-3.5 w-3.5" />
                                      )}
                                      <span>{t("wanted.scoreBreakdown")}</span>
                                    </button>
                                    {scoringExpanded ? (
                                      <WantedScoringBreakdown entries={scoringEntries} />
                                    ) : null}
                                  </div>
                                ) : null}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          )
        ) : (
          <div
            className={
              shouldScrollDesktopTable
                ? "min-h-0 flex-1 overflow-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
                : "overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
            }
          >
            <Table
              overflow="visible"
              layout="auto"
              density="dense"
              className="min-w-[1280px]"
            >
              <TableHeader
                className={shouldScrollDesktopTable ? "[&_th]:sticky [&_th]:top-0 [&_th]:z-10" : undefined}
              >
                <TableRow>
                  <TableHead className="w-10 min-w-10 text-center" />
                  <TableHead className="w-[30%] min-w-[280px]">
                    {t("wanted.colTitle")}
                  </TableHead>
                  <TableHead className="min-w-[112px] text-center">Library</TableHead>
                  <TableHead className="min-w-[80px] text-center">
                    {t("wanted.colType")}
                  </TableHead>
                  <TableHead className="min-w-[96px] text-center">
                    {t("wanted.colStatus")}
                  </TableHead>
                  <TableHead className="min-w-[128px] text-center">
                    {t("wanted.colConvergence")}
                  </TableHead>
                  <TableHead className="min-w-[144px] text-center">
                    {t("wanted.colLatestDecision")}
                  </TableHead>
                  <TableHead className="min-w-[128px] text-center">
                    {t("wanted.colLastSearch")}
                  </TableHead>
                  <TableHead className="min-w-[64px] text-center">
                    {t("wanted.colScore")}
                  </TableHead>
                  <TableHead className="min-w-[80px] text-center">
                    {t("wanted.colIndexers")}
                  </TableHead>
                  <TableActionsHead className="min-w-[120px]">
                    {t("label.actions")}
                  </TableActionsHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => (
                  <Fragment key={item.id}>
                    <TableRow
                      id={wantedItemRowId(item.id)}
                      data-ui="wanted-item-row"
                      data-wanted-item-id={item.id}
                      data-wanted-title={item.titleName ?? item.titleId}
                      className="group"
                    >
                      <TableCell className="text-center">
                        <button
                          className="p-0.5 text-muted-foreground hover:text-foreground"
                          onClick={() => void loadItemDetails(item.id, item.standbyCount)}
                        >
                          {expandedItemId === item.id ? (
                            <ChevronDown className="h-4 w-4" />
                          ) : (
                            <ChevronRight className="h-4 w-4" />
                          )}
                        </button>
                      </TableCell>
                      <TableCell
                        className="min-w-0 text-sm"
                        title={item.titleName ?? item.titleId}
                      >
                        <button
                          type="button"
                          className="block min-w-0 max-w-full text-left hover:text-foreground"
                          onClick={() => openWantedItemOverview(item)}
                        >
                          <div className="whitespace-normal break-words font-medium hover:underline">
                            {item.titleName ?? item.titleId.slice(0, 8)}
                          </div>
                          <div className="truncate text-xs text-muted-foreground">
                            {wantedItemSubtitle(item, t)}
                          </div>
                          {item.sourceProvider ? (
                            <div className="truncate text-xs text-muted-foreground">
                              {item.sourceProvider}
                            </div>
                          ) : null}
                        </button>
                        <div className="mt-1">
                          <StandbyChip
                            item={item}
                            expanded={expandedItemId === item.id}
                            onExpand={() => void loadItemDetails(item.id, item.standbyCount)}
                          />
                        </div>
                      </TableCell>
                      <TableCell className="text-center text-xs text-muted-foreground">
                        <span className="block truncate">
                          {item.libraryName ?? item.libraryId ?? "Library"}
                        </span>
                      </TableCell>
                      <TableCell className="text-center">
                        {formatWantedMediaType(item.mediaType, t)}
                      </TableCell>
                      <TableCell className="text-center">
                        {statusBadge(item.status, t)}
                      </TableCell>
                      <TableCell className="text-center">
                        <ConvergenceBadge
                          state={item.convergenceState}
                          indexersCovered={item.indexersCovered}
                          indexersRouted={item.indexersRouted}
                          recencyLane={item.recencyLane}
                        />
                        <NoStandbyCandidates item={item} />
                      </TableCell>
                      <TableCell className="text-center text-xs">
                        {item.latestReleaseDecision ? (
                          <div className="space-y-1">
                            {decisionBadge(item.latestReleaseDecision.decisionCode, t)}
                            <div className="text-muted-foreground">
                              {formatDate(
                                item.latestReleaseDecision.createdAt,
                                dateTimeFormat,
                              )}
                            </div>
                          </div>
                        ) : (
                          "—"
                        )}
                      </TableCell>
                      <TableCell className="text-center text-xs">
                        {formatDate(item.lastSearchAt, dateTimeFormat)}
                      </TableCell>
                      <TableCodeCell className="text-center">
                        {item.currentScore ?? "—"}
                      </TableCodeCell>
                      <TableCodeCell className="text-center">
                        {item.indexersCovered}/{item.indexersRouted}
                      </TableCodeCell>
                      <TableActionsCell className="min-w-[120px]">
                        <div className="flex flex-wrap justify-center gap-1">
                          <IconButton
                            id={wantedItemSearchNowId(item.id)}
                            data-ui="wanted-item-search-now"
                            data-wanted-item-id={item.id}
                            label={t("wanted.searchNow")}
                            appearance="ghost"
                            className="h-7 w-7"
                            onClick={() => void triggerSearch(item.id)}
                          >
                            <Search className="h-3.5 w-3.5" />
                          </IconButton>
                          {item.status === "PAUSED" ? (
                            <IconButton
                              label={t("wanted.resume")}
                              appearance="ghost"
                              className="h-7 w-7"
                              onClick={() => void resumeItem(item.id)}
                            >
                              <Play className="h-3.5 w-3.5" />
                            </IconButton>
                          ) : (
                            <IconButton
                              label={t("wanted.pause")}
                              appearance="ghost"
                              className="h-7 w-7"
                              onClick={() => void pauseItem(item.id)}
                            >
                              <Pause className="h-3.5 w-3.5" />
                            </IconButton>
                          )}
                          {item.mismatchRecoveryEligible ? (
                            <IconButton
                              label={t("wanted.actionRecoverMismatch")}
                              appearance="ghost"
                              className="h-7 w-7"
                              onClick={() => void triggerMismatchRecovery(item.titleId)}
                            >
                              <RefreshCw className="h-3.5 w-3.5" />
                            </IconButton>
                          ) : null}
                        </div>
                      </TableActionsCell>
                    </TableRow>
                    {expandedItemId === item.id && (
                      <TableRow>
                        <TableCell colSpan={11} className="bg-muted/30 p-4">
                          <StandbyReleasesList items={standbyReleases} loading={standbyLoading} />
                          {decisionsLoading ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.loadingDecisions")}
                            </p>
                          ) : decisions.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.noDecisions")}
                            </p>
                          ) : (
                            <Table
                              overflow="clip"
                              layout="fixed"
                              density="dense"
                            >
                              <TableHeader>
                                <TableRow>
                                  <TableHead className="w-10 text-center" />
                                  <TableHead>{t("wanted.decRelease")}</TableHead>
                                  <TableHead className="w-28 text-center">
                                    {t("wanted.decDecision")}
                                  </TableHead>
                                  <TableHead className="w-24 text-center">
                                    {t("wanted.decScore")}
                                  </TableHead>
                                  <TableHead className="w-24 text-center">
                                    {t("wanted.decDelta")}
                                  </TableHead>
                                  <TableHead className="w-28 text-center">
                                    {t("wanted.decSize")}
                                  </TableHead>
                                  <TableHead className="w-32 text-center">
                                    {t("wanted.decDate")}
                                  </TableHead>
                                </TableRow>
                              </TableHeader>
                              <TableBody>
                                {decisions.map((d) => {
                                  const scoringEntries = parseDecisionExplanation(d.explanationJson);
                                  const hasScoringBreakdown = scoringEntries.length > 0;
                                  const scoringExpanded = expandedDecisionIds.has(d.id);

                                  return (
                                    <Fragment key={d.id}>
                                      <TableRow>
                                        <TableCell className="text-center">
                                          {hasScoringBreakdown ? (
                                            <button
                                              type="button"
                                              className="p-0.5 text-muted-foreground hover:text-foreground"
                                              onClick={() => toggleDecisionScoring(d.id)}
                                            >
                                              {scoringExpanded ? (
                                                <ChevronDown className="h-4 w-4" />
                                              ) : (
                                                <ChevronRight className="h-4 w-4" />
                                              )}
                                            </button>
                                          ) : null}
                                        </TableCell>
                                        <TableCell
                                          className="truncate text-xs"
                                          title={d.releaseTitle}
                                        >
                                          {d.releaseTitle}
                                        </TableCell>
                                        <TableCell className="text-center">
                                          {decisionBadge(d.decisionCode, t)}
                                        </TableCell>
                                        <TableCodeCell className="text-center">
                                          {d.candidateScore}
                                        </TableCodeCell>
                                        <TableCodeCell className="text-center">
                                          {d.scoreDelta ?? "—"}
                                        </TableCodeCell>
                                        <TableCodeCell className="text-center text-xs">
                                          {formatBytes(d.releaseSizeBytes)}
                                        </TableCodeCell>
                                        <TableCell className="text-center text-xs">
                                          {formatDate(d.createdAt, dateTimeFormat)}
                                        </TableCell>
                                      </TableRow>
                                      {scoringExpanded ? (
                                        <TableRow>
                                          <TableCell colSpan={7} className="bg-background/70 p-3">
                                            <WantedScoringBreakdown entries={scoringEntries} />
                                          </TableCell>
                                        </TableRow>
                                      ) : null}
                                    </Fragment>
                                  );
                                })}
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
                    <TableCell colSpan={11} className="text-center text-muted-foreground">
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

function formatTimeRemaining(delayUntil: string, t: Translate): string {
  const target = new Date(delayUntil).getTime();
  const now = Date.now();
  const diff = target - now;
  if (diff <= 0) return t("wanted.timeNow");
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const minutes = Math.floor((diff % (1000 * 60 * 60)) / (1000 * 60));
  if (hours > 0) {
    return t("wanted.timeHoursMinutes", { hours, minutes });
  }
  return t("wanted.timeMinutes", { minutes });
}

function formatPendingStatus(status: PendingReleaseStatus, t: Translate): string {
  if (status === "NEEDS_REVIEW") {
    return t("pending.status.needsReview");
  }
  return status
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function pendingStatusBadge(status: PendingReleaseStatus, t: Translate) {
  const cls =
    status === "GRABBED"
      ? "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
      : status === "EXPIRED" || status === "DISMISSED"
        ? "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]"
        : status === "PROCESSING"
          ? "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]"
          : status === "SUPERSEDED" || status === "NEEDS_REVIEW"
            ? "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
            : "border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-muted2)]";
  return (
    <span className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {formatPendingStatus(status, t)}
    </span>
  );
}

function pendingPhaseBadge(status: PendingReleaseStatus, t: Translate) {
  const label =
    status === "PROCESSING"
      ? "Processing"
      : status === "GRABBED"
        ? "Grabbed"
        : status === "EXPIRED" || status === "DISMISSED"
          ? "Closed"
          : status === "SUPERSEDED"
            ? "Superseded"
            : status === "NEEDS_REVIEW"
              ? t("pending.phase.needsReview")
              : "Scheduled";
  return (
    <span className="inline-flex rounded-full border border-[rgba(var(--scry-accent-rgb),0.24)] bg-[rgba(var(--scry-accent-rgb),0.13)] px-2 py-0.5 text-xs font-medium text-[var(--scry-accent-text)]">
      {label}
    </span>
  );
}

function PendingReleasesCard({ state }: { state: PendingViewState }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMobile = useIsMobile();
  const loadMoreRef = useRef<HTMLDivElement | null>(null);
  const {
    items,
    loading,
    hasMore,
    loadingMore,
    refreshItems,
    loadMoreItems,
    forceGrab,
    dismiss,
  } = state;
  const [expandedPendingId, setExpandedPendingId] = useState<string | null>(null);

  const togglePendingExpanded = (id: string) => {
    setExpandedPendingId((current) => (current === id ? null : id));
  };

  useEffect(() => {
    const node = loadMoreRef.current;
    if (!node || !hasMore || loadingMore) {
      return;
    }

    if (typeof IntersectionObserver === "undefined") {
      void loadMoreItems();
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          void loadMoreItems();
        }
      },
      { rootMargin: "900px 0px" },
    );
    observer.observe(node);
    return () => {
      observer.disconnect();
    };
  }, [hasMore, items.length, loadMoreItems, loadingMore]);

  return (
    <Card className="overflow-hidden rounded-none border-0 bg-transparent shadow-none">
      <CardHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-end">
          <Button
            className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)] sm:w-auto"
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
      <CardContent className="bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5">
        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("pending.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => {
                const expanded = expandedPendingId === item.id;
                return (
                <div
                  key={item.id}
                  id={selectorId("pending-release", "card", item.id)}
                  data-ui="pending-release-row"
                  data-release-title={item.releaseTitle}
                  data-pending-status={item.status}
                  className="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]"
                >
                  <div className="flex items-start gap-2">
                    <button
                      type="button"
                      className="mt-0.5 p-0.5 text-muted-foreground hover:text-foreground"
                      onClick={() => togglePendingExpanded(item.id)}
                    >
                      {expanded ? (
                        <ChevronDown className="h-4 w-4" />
                      ) : (
                        <ChevronRight className="h-4 w-4" />
                      )}
                    </button>
                    <div className="min-w-0 flex-1">
                      <p className="break-words text-sm font-medium text-foreground">{item.releaseTitle}</p>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {pendingStatusBadge(item.status, t)}
                        {pendingPhaseBadge(item.status, t)}
                      </div>
                    </div>
                  </div>
                  <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <div>
                      <span className="block">{t("pending.colScore")}</span>
                      <span className="text-foreground">{item.releaseScore}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colSize")}</span>
                      <span className="font-[var(--font-code)] text-foreground">
                        {item.releaseSizeBytes == null ? "—" : formatBytes(item.releaseSizeBytes)}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colIndexer")}</span>
                      <span className="text-foreground">{item.indexerSource ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colDelayUntil")}</span>
                      <span
                        className="text-foreground"
                        title={formatDate(item.delayUntil, dateTimeFormat)}
                      >
                        {formatTimeRemaining(item.delayUntil, t)}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colReason")}</span>
                      <span className="text-foreground">{item.lastDecisionCode ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colRole")}</span>
                      <span className="text-foreground">{item.role === "PRIMARY" ? t("pending.role.primary") : t("pending.role.fallback")}</span>
                    </div>
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {formatDate(item.addedAt, dateTimeFormat)}
                  </p>
                  {expanded ? (
                    <div className="mt-3 grid gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-bg)] p-3 text-xs">
                      <div>
                        <span className="block text-muted-foreground">Title ID</span>
                        <span className="break-all text-foreground">{item.titleId}</span>
                      </div>
                      <div>
                        <span className="block text-muted-foreground">Wanted Item</span>
                        <span className="break-all text-foreground">{item.wantedItemId}</span>
                      </div>
                    </div>
                  ) : null}
                  <div className="mt-3 flex gap-2">
                    <Button
                      size="sm"
                      variant="secondary"
                      id={selectorId("pending-release", "force-grab-card", item.id)}
                      className="flex-1"
                      onClick={() => void forceGrab(item.id)}
                    >
                      <Download className="h-4 w-4" />
                      <span>{t("pending.forceGrab")}</span>
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      id={selectorId("pending-release", "dismiss-card", item.id)}
                      className="flex-1"
                      onClick={() => void dismiss(item.id)}
                    >
                      <X className="h-4 w-4" />
                      <span>{t("pending.dismiss")}</span>
                    </Button>
                  </div>
                </div>
                );
              })}
            </div>
          )
        ) : (
          <div className="overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]">
            <Table overflow="clip" layout="fixed" density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-10 text-center" />
                  <TableHead>{t("pending.colRelease")}</TableHead>
                  <TableHead className="w-28 text-center">{t("wanted.colStatus")}</TableHead>
                  <TableHead className="w-28 text-center">{t("wanted.colPhase")}</TableHead>
                  <TableHead className="w-20 text-center">{t("pending.colScore")}</TableHead>
                  <TableHead className="w-28 text-center">{t("pending.colSize")}</TableHead>
                  <TableHead className="w-32 text-center">{t("pending.colIndexer")}</TableHead>
                  <TableHead className="w-32 text-center">{t("pending.colAddedAt")}</TableHead>
                  <TableHead className="w-32 text-center">{t("pending.colDelayUntil")}</TableHead>
                  <TableHead className="w-36 text-center">{t("pending.colReason")}</TableHead>
                  <TableHead className="w-24 text-center">{t("pending.colRole")}</TableHead>
                  <TableActionsHead className="w-24">{t("label.actions")}</TableActionsHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => {
                  const expanded = expandedPendingId === item.id;
                  return (
                    <Fragment key={item.id}>
                      <TableRow
                        id={selectorId("pending-release", "row", item.id)}
                        data-ui="pending-release-row"
                        data-release-title={item.releaseTitle}
                        data-pending-status={item.status}
                      >
                        <TableCell className="text-center">
                          <button
                            type="button"
                            id={selectorId("pending-release", "expand", item.id)}
                            className="p-0.5 text-muted-foreground hover:text-foreground"
                            onClick={() => togglePendingExpanded(item.id)}
                          >
                            {expanded ? (
                              <ChevronDown className="h-4 w-4" />
                            ) : (
                              <ChevronRight className="h-4 w-4" />
                            )}
                          </button>
                        </TableCell>
                        <TableCell className="truncate text-sm" title={item.releaseTitle}>
                          {item.releaseTitle}
                        </TableCell>
                        <TableCell className="text-center">{pendingStatusBadge(item.status, t)}</TableCell>
                        <TableCell className="text-center">{pendingPhaseBadge(item.status, t)}</TableCell>
                        <TableCodeCell className="text-center">{item.releaseScore}</TableCodeCell>
                        <TableCodeCell className="text-center text-xs">
                          {item.releaseSizeBytes == null ? "—" : formatBytes(item.releaseSizeBytes)}
                        </TableCodeCell>
                        <TableCell className="text-center text-xs">{item.indexerSource ?? "—"}</TableCell>
                        <TableCell className="text-center text-xs">
                          {formatDate(item.addedAt, dateTimeFormat)}
                        </TableCell>
                        <TableCell className="text-center text-xs">
                          <span title={formatDate(item.delayUntil, dateTimeFormat)}>
                            {formatTimeRemaining(item.delayUntil, t)}
                          </span>
                        </TableCell>
                        <TableCell className="truncate text-center text-xs" title={item.lastDecisionCode ?? undefined}>
                          {item.lastDecisionCode ?? "—"}
                        </TableCell>
                        <TableCell className="text-center text-xs">
                          {item.role === "PRIMARY" ? t("pending.role.primary") : t("pending.role.fallback")}
                        </TableCell>
                        <TableActionsCell className="w-24">
                          <div className="flex justify-center gap-1">
                            <IconButton
                              id={selectorId("pending-release", "force-grab", item.id)}
                              label={t("pending.forceGrab")}
                              appearance="ghost"
                              className="h-7 w-7"
                              onClick={() => void forceGrab(item.id)}
                            >
                              <Download className="h-3.5 w-3.5" />
                            </IconButton>
                            <IconButton
                              id={selectorId("pending-release", "dismiss", item.id)}
                              label={t("pending.dismiss")}
                              appearance="ghost"
                              className="h-7 w-7"
                              onClick={() => void dismiss(item.id)}
                            >
                              <X className="h-3.5 w-3.5" />
                            </IconButton>
                          </div>
                        </TableActionsCell>
                      </TableRow>
                      {expanded ? (
                        <TableRow>
                          <TableCell colSpan={12} className="bg-background/30 p-4">
                            <div className="grid gap-3 text-xs sm:grid-cols-2 lg:grid-cols-4">
                              <div>
                                <span className="block text-muted-foreground">Title ID</span>
                                <span className="break-all text-foreground">{item.titleId}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">Wanted Item</span>
                                <span className="break-all text-foreground">{item.wantedItemId}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colAddedAt")}</span>
                                <span className="text-foreground">{formatDate(item.addedAt, dateTimeFormat)}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colDelayUntil")}</span>
                                <span className="text-foreground">{formatDate(item.delayUntil, dateTimeFormat)}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colReason")}</span>
                                <span className="text-foreground">{item.lastDecisionCode ?? "—"}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colRole")}</span>
                                <span className="text-foreground">{item.role === "PRIMARY" ? t("pending.role.primary") : t("pending.role.fallback")}</span>
                              </div>
                            </div>
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </Fragment>
                  );
                })}
                {items.length === 0 && !loading && (
                  <TableRow>
                    <TableCell colSpan={12} className="text-center text-muted-foreground">
                      {t("pending.noItems")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}
        <div ref={loadMoreRef} aria-hidden="true" className="h-px" />
        {loadingMore ? (
          <p className="mt-3 text-center text-sm text-muted-foreground">
            {t("wanted.refreshing")}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}

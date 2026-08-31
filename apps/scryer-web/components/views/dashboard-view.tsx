import * as React from "react";
import {
  ArrowDownToLine,
  ActivitySquare,
  ArrowUp,
  CircleAlert,
  Database,
  Download,
  FileVideo,
  FolderInput,
  HardDrive,
  Inbox,
  Puzzle,
  Trash2,
  TriangleAlert,
  X,
  Check,
} from "lucide-react";
import { Link } from "react-router";

import { AuthenticatedAvatar } from "@/components/common/authenticated-avatar";
import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import {
  IndexerErrorHistoryModal,
  type IndexerErrorHistoryScope,
} from "@/components/common/indexer-error-history-modal";
import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  DashboardPanel,
  DashboardPanelEmpty,
} from "@/components/views/dashboard/dashboard-panel";
import { StorageUsageRing } from "@/components/views/dashboard/storage-usage-ring";
import {
  usageTagBadgeTone,
  usageTagLabelKey,
  usageToneStyle,
} from "@/components/views/dashboard/usage-tone-style";
import { facetPillStyle } from "@/components/setup/import/facet-style";
import { TitlePoster } from "@/components/title-poster";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { IconButton } from "@/components/ui/icon-button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { facetById } from "@/lib/facets/registry";
import type {
  DashboardImportedItem,
  DashboardIndexerStat,
  DashboardOverview,
  DashboardPluginUpdate,
  DashboardRequest,
  DashboardStorageRoot,
  DownloadQueueItem,
  Facet,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  APP_PERMISSIONS,
  hasAnyAppPermission,
} from "@/lib/utils/permissions";
import {
  deriveQueueRowPresentation,
  formatBytes,
  getProgressBarColor,
} from "@/lib/utils/activity-utils";
import {
  aggregateClientActivity,
  attentionTotal,
  compareProviderRows,
  formatCompactAge,
  formatTerabytes,
  groupStorageRootsByLibrary,
  isProviderErroring,
  summarizeIndexerHealth,
  usagePercent,
  usageTone,
} from "@/lib/utils/dashboard";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { buildViewPath } from "@/lib/utils/routing";

/** Rows visible before the top panels start scrolling. */
const PREVIEW_PANE_CLASS = "max-h-[172px] overflow-y-auto";
const QUEUE_PREVIEW_LIMIT = 5;
/**
 * Provider tables show about five rows before scrolling. This goes on the
 * table's own wrapper (`wrapperClassName`), not the panel body: `Table` already
 * makes that wrapper a scroll container, and a sticky header only sticks to the
 * scroller it lives inside.
 */
const TABLE_PANE_CLASS = "max-h-[218px] overflow-y-auto";
/**
 * Header stays put while rows scroll. `sticky` goes on the header *cells*, not
 * the header row: a sticky `<tr>` is constrained to its `<thead>`, which is one
 * row tall, so it has no travel and scrolls away with the body. A `<th>` is
 * contained by the table itself and pins for the table's full height. The
 * opaque background is what the rows scroll under.
 */
const TABLE_HEAD_CLASS =
  "sticky top-0 z-10 h-8 bg-card px-3 text-[10px] uppercase";
const TABLE_HEAD_RIGHT_CLASS =
  "sticky top-0 z-10 h-8 bg-card px-3 text-right text-[10px] uppercase";

export type DashboardViewProps = {
  loading: boolean;
  overview: DashboardOverview | null;
  requests: DashboardRequest[];
  importActivity: DownloadQueueItem[];
  importActivityTotal: number;
  recentImports: DashboardImportedItem[];
  queueItems: DownloadQueueItem[];
  queueTotal: number;
  pluginUpdates: DashboardPluginUpdate[];
  updatingPluginIds: string[];
  actionRequestId: string | null;
  /** Import-activity row currently running an action, disabling its buttons. */
  importActionItemId: string | null;
  onApproveRequest: (request: DashboardRequest) => void;
  onDismissRequest: (request: DashboardRequest) => void;
  onImportItem: (item: DownloadQueueItem) => void;
  onMarkImportFailed: (item: DownloadQueueItem) => void;
  onRemoveImportItem: (item: DownloadQueueItem) => void;
  onUpdatePlugin: (pluginId: string) => void;
  onUpdateAllPlugins: () => void;
};

export function DashboardView({
  loading,
  overview,
  requests,
  importActivity,
  importActivityTotal,
  recentImports,
  queueItems,
  queueTotal,
  pluginUpdates,
  updatingPluginIds,
  actionRequestId,
  importActionItemId,
  onApproveRequest,
  onDismissRequest,
  onImportItem,
  onMarkImportFailed,
  onRemoveImportItem,
  onUpdatePlugin,
  onUpdateAllPlugins,
}: DashboardViewProps) {
  const indexerHealth = React.useMemo(
    () => summarizeIndexerHealth(overview?.indexers ?? []),
    [overview?.indexers],
  );

  if (loading && !overview) {
    return <DashboardSkeleton />;
  }

  return (
    <div className="flex w-full flex-col gap-3 px-5 pb-10 pt-4">
      <DashboardHeader
        username={overview?.username ?? null}
        requestCount={overview?.pendingRequestCount ?? 0}
        importCount={overview?.activityImportCount ?? 0}
        pluginCount={pluginUpdates.length}
        indexerErrorCount={indexerHealth.erroring}
      />

      <PluginUpdateStrip
        updates={pluginUpdates}
        updatingPluginIds={updatingPluginIds}
        onUpdatePlugin={onUpdatePlugin}
        onUpdateAllPlugins={onUpdateAllPlugins}
      />

      <StatsRow overview={overview} />

      <div className="grid grid-cols-1 gap-3 min-[1241px]:grid-cols-2 min-[1501px]:grid-cols-3">
        <RequestsPanel
          requests={requests}
          totalCount={overview?.pendingRequestCount ?? requests.length}
          actionRequestId={actionRequestId}
          onApprove={onApproveRequest}
          onDismiss={onDismissRequest}
        />
        <ManualImportsPanel
          items={importActivity}
          totalCount={importActivityTotal}
          busyItemId={importActionItemId}
          onImport={onImportItem}
          onMarkFailed={onMarkImportFailed}
          onRemove={onRemoveImportItem}
        />
        <RecentlyImportedPanel items={recentImports} />
      </div>

      <div className="grid grid-cols-1 gap-3 min-[1241px]:grid-cols-2">
        <IndexersPanel overview={overview} />
        <DownloadClientsPanel overview={overview} queueItems={queueItems} />
      </div>

      <div className="grid grid-cols-1 gap-3 min-[1241px]:grid-cols-2">
        <StoragePanel overview={overview} />
        <ActiveQueuePanel items={queueItems} totalCount={queueTotal} />
      </div>
    </div>
  );
}

// ── Header ──────────────────────────────────────────────────────────────────

function DashboardHeader({
  username,
  requestCount,
  importCount,
  pluginCount,
  indexerErrorCount,
}: {
  username: string | null;
  requestCount: number;
  importCount: number;
  pluginCount: number;
  indexerErrorCount: number;
}) {
  const t = useTranslate();
  const total = attentionTotal({
    requests: requestCount,
    imports: importCount,
    pluginUpdates: pluginCount,
    indexerErrors: indexerErrorCount,
  });

  const sources = [
    requestCount > 0 ? t("dashboard.sourceRequests") : null,
    importCount > 0 ? t("dashboard.sourceImports") : null,
    pluginCount > 0 ? t("dashboard.sourcePlugins") : null,
    indexerErrorCount > 0 ? t("dashboard.sourceIndexers") : null,
  ].filter((value): value is string => value !== null);

  return (
    <header className="flex min-w-0 flex-col gap-1">
      <h1 className="font-[var(--font-display)] text-[19px] font-bold text-[var(--scry-ink2)]">
        {t(greetingKey(), { name: username ?? "" })}
      </h1>
      <p className="text-[12px] text-[var(--scry-muted)]">
        {total === 0
          ? t("dashboard.allClear")
          : `${
              total === 1
                ? t("dashboard.attentionCountOne")
                : t("dashboard.attentionCount", { count: total })
            } · ${sources.join(", ")}`}
      </p>
    </header>
  );
}

/** Greeting keyed off the viewer's own clock. */
function greetingKey(): string {
  const hour = new Date().getHours();
  if (hour < 12) {
    return "dashboard.greetingMorning";
  }
  if (hour < 18) {
    return "dashboard.greetingAfternoon";
  }
  return "dashboard.greetingEvening";
}

// ── Plugin updates ──────────────────────────────────────────────────────────

function PluginUpdateStrip({
  updates,
  updatingPluginIds,
  onUpdatePlugin,
  onUpdateAllPlugins,
}: {
  updates: DashboardPluginUpdate[];
  updatingPluginIds: string[];
  onUpdatePlugin: (pluginId: string) => void;
  onUpdateAllPlugins: () => void;
}) {
  const t = useTranslate();
  // The strip must occupy no space at all when nothing needs updating.
  if (updates.length === 0) {
    return null;
  }

  const busy = updatingPluginIds.length > 0;

  return (
    <section
      data-slot="dashboard-plugin-strip"
      className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2"
    >
      <Puzzle
        className="h-4 w-4 shrink-0 text-[var(--scry-warning-text)]"
        aria-hidden="true"
      />
      <span className="shrink-0 whitespace-nowrap text-[12px] font-semibold text-[var(--scry-warning-text)]">
        {updates.length === 1
          ? t("dashboard.pluginUpdateCountOne")
          : t("dashboard.pluginUpdateCount", { count: updates.length })}
      </span>

      <ul className="hidden min-w-0 flex-wrap items-center gap-2 min-[761px]:flex">
        {updates.map((update) => (
          <li
            key={update.id}
            className="flex min-w-0 items-center gap-1.5 rounded-md border border-[var(--scry-warning-border)] px-2 py-0.5 text-[11px]"
          >
            <span className="truncate text-[var(--scry-ink2)]">{update.name}</span>
            <span className="whitespace-nowrap font-[var(--font-code)] text-[var(--scry-muted2)]">
              {update.fromVersion ?? "—"}
            </span>
            <ArrowUp className="h-3 w-3 shrink-0 text-[var(--scry-muted2)]" aria-hidden="true" />
            <span className="whitespace-nowrap font-[var(--font-code)] text-[var(--scry-success-text)]">
              {update.toVersion ?? "—"}
            </span>
            {update.breaking ? (
              <Badge tone="negative" className="px-1 py-0 text-[10px] uppercase">
                {t("dashboard.pluginBreaking")}
              </Badge>
            ) : null}
            <IconButton
              appearance="ghost"
              tone="upgrade"
              label={`${t("label.update")} ${update.name}`}
              disabled={updatingPluginIds.includes(update.id)}
              onClick={() => onUpdatePlugin(update.id)}
            >
              <ArrowUp className="h-3 w-3" aria-hidden="true" />
            </IconButton>
          </li>
        ))}
      </ul>

      <div className="ml-auto flex shrink-0 items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={busy}
          onClick={onUpdateAllPlugins}
        >
          {t("dashboard.pluginUpdateAll")}
        </Button>
        <Link
          to={buildViewPath("settings", "plugins")}
          className="whitespace-nowrap text-[11px] font-medium text-[var(--scry-warning-text)] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] focus-visible:ring-offset-0"
        >
          {t("dashboard.pluginManage")}
        </Link>
      </div>
    </section>
  );
}

// ── Stats ───────────────────────────────────────────────────────────────────

function StatsRow({ overview }: { overview: DashboardOverview | null }) {
  const t = useTranslate();
  const activity = overview?.activity;
  const grabbedDelta = activity
    ? activity.current.grabbed - activity.previous.grabbed
    : 0;
  const importedDelta = activity
    ? activity.current.imported - activity.previous.imported
    : 0;

  return (
    <div className="grid grid-cols-1 gap-3 min-[701px]:grid-cols-2 min-[1081px]:grid-cols-[1.25fr_1fr_1fr]">
      <StatTile
        label={t("dashboard.statLibrary")}
        metrics={[
          {
            key: "movies",
            value: overview?.library.movies ?? 0,
            label: t("dashboard.metricMovies"),
            facet: "MOVIE",
          },
          {
            key: "series",
            value: overview?.library.series ?? 0,
            label: t("dashboard.metricSeries"),
            facet: "SERIES",
          },
          {
            key: "anime",
            value: overview?.library.anime ?? 0,
            label: t("dashboard.metricAnime"),
            facet: "ANIME",
          },
        ]}
      />
      <StatTile
        label={t("dashboard.statWindow")}
        delta={grabbedDelta}
        metrics={[
          {
            key: "queued",
            value: activity?.current.grabbed ?? 0,
            label: t("dashboard.metricQueued"),
          },
          {
            key: "upgrades",
            value: activity?.current.upgraded ?? 0,
            label: t("dashboard.metricUpgrades"),
          },
        ]}
      />
      <StatTile
        label={t("dashboard.statWindow")}
        delta={importedDelta}
        metrics={[
          {
            key: "imported",
            value: activity?.current.imported ?? 0,
            label: t("dashboard.metricImported"),
          },
          {
            key: "failed",
            value: activity?.current.importFailed ?? 0,
            label: t("dashboard.metricFailed"),
            tone: "danger",
          },
        ]}
      />
    </div>
  );
}

type StatMetric = {
  key: string;
  value: number;
  label: string;
  facet?: Facet;
  tone?: "danger";
};

function StatTile({
  label,
  metrics,
  delta,
}: {
  label: string;
  metrics: StatMetric[];
  delta?: number;
}) {
  const t = useTranslate();

  return (
    <Card className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2 px-3 py-2">
      <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-[var(--scry-muted2)]">
        {label}
      </span>
      {metrics.map((metric) => {
        const facet = metric.facet;
        const Icon = facet ? facetById(facet)?.icon : null;
        return (
          <span key={metric.key} className="flex shrink-0 items-center gap-1.5">
            {Icon && facet ? (
              <Icon
                className="h-3.5 w-3.5"
                style={{ color: facetPillStyle(facet).color }}
                aria-hidden="true"
              />
            ) : null}
            <span
              className={cn(
                "font-[var(--font-display)] text-[18px] font-bold tabular-nums",
                metric.tone === "danger" && metric.value > 0
                  ? "text-[var(--scry-danger-text)]"
                  : "text-[var(--scry-ink2)]",
              )}
            >
              {metric.value.toLocaleString()}
            </span>
            {/* `shrink-0` is load-bearing: without it these labels collapse to
                zero width and the tile reads as bare numbers. */}
            <span className="shrink-0 text-[11px] text-[var(--scry-muted)]">
              {metric.label}
            </span>
          </span>
        );
      })}
      {typeof delta === "number" && delta !== 0 ? (
        <span
          title={t("dashboard.deltaHint")}
          className={cn(
            "ml-auto shrink-0 whitespace-nowrap text-[11px] font-semibold tabular-nums",
            delta > 0
              ? "text-[var(--scry-success-text)]"
              : "text-[var(--scry-danger-text)]",
          )}
        >
          {delta > 0 ? `+${delta}` : delta}
        </span>
      ) : null}
    </Card>
  );
}

// ── Requests ────────────────────────────────────────────────────────────────

function RequestsPanel({
  requests,
  totalCount,
  actionRequestId,
  onApprove,
  onDismiss,
}: {
  requests: DashboardRequest[];
  totalCount: number;
  actionRequestId: string | null;
  onApprove: (request: DashboardRequest) => void;
  onDismiss: (request: DashboardRequest) => void;
}) {
  const t = useTranslate();

  return (
    <DashboardPanel
      icon={Inbox}
      title={t("nav.requests")}
      count={totalCount}
      linkTo="/requests"
      linkLabel={t("dashboard.viewAll")}
      bodyClassName={PREVIEW_PANE_CLASS}
    >
      {requests.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyRequests")} />
      ) : (
        <ul>
          {requests.map((request) => {
            const requester = request.requesters[0];
            const busy = actionRequestId === request.id;
            return (
              <li
                key={request.id}
                className="flex min-w-0 items-center gap-2 border-b border-border px-3 py-[7px] last:border-b-0"
              >
                <RowPoster posterUrl={request.posterUrl} facet={request.facet} />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[12px] font-medium text-[var(--scry-ink2)]">
                    {request.title}
                    {request.year ? (
                      <span className="ml-1 text-[var(--scry-muted2)]">
                        {request.year}
                      </span>
                    ) : null}
                  </span>
                  <span className="flex min-w-0 items-center gap-1.5">
                    {requester ? (
                      <span className="flex min-w-0 items-center gap-1">
                        <AuthenticatedAvatar
                          avatarUrl={requester.avatarUrl}
                          label={requester.username}
                          imageClassName="h-3.5 w-3.5 rounded-full object-cover"
                          fallbackClassName="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-muted text-[8px] text-muted-foreground"
                        />
                        <span className="truncate text-[11px] text-[var(--scry-muted2)]">
                          {requester.username}
                        </span>
                      </span>
                    ) : null}
                    <FacetChip facet={request.facet} />
                  </span>
                </span>
                <AgeLabel isoDate={request.createdAt} />
                <IconButton
                  appearance="boxed"
                  tone="enabled"
                  disabled={busy}
                  label={t("dashboard.approveRequest", { name: request.title })}
                  onClick={() => onApprove(request)}
                >
                  <Check className="h-3.5 w-3.5" aria-hidden="true" />
                </IconButton>
                <IconButton
                  appearance="boxed"
                  tone="neutral"
                  disabled={busy}
                  label={t("dashboard.dismissRequest", { name: request.title })}
                  onClick={() => onDismiss(request)}
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </IconButton>
              </li>
            );
          })}
        </ul>
      )}
    </DashboardPanel>
  );
}

function RowPoster({
  posterUrl,
  facet,
}: {
  posterUrl: string | null;
  facet: Facet | null;
}) {
  const [failed, setFailed] = React.useState(false);
  const variantUrl = selectPosterVariantUrl(posterUrl, "w70");
  const Icon = facet ? facetById(facet)?.icon : null;

  if (!variantUrl || failed) {
    return (
      <span className="flex h-[35px] w-6 shrink-0 items-center justify-center rounded-[3px] bg-[var(--scry-chip)]">
        {Icon ? (
          <Icon className="h-3 w-3 text-[var(--scry-muted2)]" aria-hidden="true" />
        ) : null}
      </span>
    );
  }

  return (
    <TitlePoster
      src={variantUrl}
      alt=""
      className="h-[35px] w-6 shrink-0 rounded-[3px] object-cover"
      onError={() => setFailed(true)}
    />
  );
}

// ── Manual imports ──────────────────────────────────────────────────────────

function ManualImportsPanel({
  items,
  totalCount,
  busyItemId,
  onImport,
  onMarkFailed,
  onRemove,
}: {
  items: DownloadQueueItem[];
  totalCount: number;
  busyItemId: string | null;
  onImport: (item: DownloadQueueItem) => void;
  onMarkFailed: (item: DownloadQueueItem) => void;
  onRemove: (item: DownloadQueueItem) => void;
}) {
  const t = useTranslate();

  return (
    <DashboardPanel
      icon={FolderInput}
      title={t("dashboard.manualImports")}
      count={totalCount}
      linkTo="/activity/import"
      linkLabel={t("dashboard.viewAll")}
      bodyClassName={PREVIEW_PANE_CLASS}
    >
      {items.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyImports")} />
      ) : (
        <ul>
          {items.map((item) => (
            <ImportActivityRow
              key={item.id}
              item={item}
              busy={busyItemId === item.id}
              onImport={onImport}
              onMarkFailed={onMarkFailed}
              onRemove={onRemove}
            />
          ))}
        </ul>
      )}
    </DashboardPanel>
  );
}

function ImportActivityRow({
  item,
  busy,
  onImport,
  onMarkFailed,
  onRemove,
}: {
  item: DownloadQueueItem;
  busy: boolean;
  onImport: (item: DownloadQueueItem) => void;
  onMarkFailed: (item: DownloadQueueItem) => void;
  onRemove: (item: DownloadQueueItem) => void;
}) {
  const t = useTranslate();
  // The same derivation Activity → Imports uses, so the dashboard can never
  // disagree with that page about what an import is doing or why it stopped.
  const presentation = deriveQueueRowPresentation(item, t);
  const failed =
    item.attentionRequired ||
    presentation.displayStateKey === "IMPORT_FAILED" ||
    presentation.displayStateKey === "FAILED";
  const blocked = presentation.displayStateKey === "IMPORT_BLOCKED";
  const StateIcon = failed ? TriangleAlert : FileVideo;
  const stateClass = failed
    ? "text-[var(--scry-danger-text)]"
    : blocked
      ? "text-[var(--scry-warning-text)]"
      : "text-[var(--scry-muted)]";

  return (
    <li className="flex min-w-0 items-center gap-2 border-b border-border px-3 py-[7px] last:border-b-0">
      <StateIcon
        className={cn("h-4 w-4 shrink-0", stateClass)}
        aria-hidden="true"
      />
      <span className="flex min-w-0 flex-1 flex-col">
        <span
          className="truncate text-[12px] font-medium text-[var(--scry-ink2)]"
          title={presentation.releaseTitle || presentation.displayTitle}
        >
          {presentation.displayTitle}
        </span>
        <span className="flex items-center gap-1.5 text-[11px]">
          <span
            className={stateClass}
            title={presentation.failureReason || undefined}
          >
            {presentation.statusLabel}
          </span>
          <span className="text-[var(--scry-muted2)] tabular-nums">
            {formatBytes(item.sizeBytes)}
          </span>
        </span>
      </span>
      <AgeLabel isoDate={item.queuedAt ?? item.lastUpdatedAt} />
      <span className="flex shrink-0 items-center gap-1">
        {presentation.canInteractiveManualImport ||
        presentation.canDirectManualImport ? (
          <IconButton
            tone="accent"
            label={t("queue.manualImportTooltip")}
            disabled={busy}
            onClick={() => onImport(item)}
          >
            <ArrowDownToLine className="h-3.5 w-3.5" aria-hidden="true" />
          </IconButton>
        ) : null}
        {presentation.canMarkFailed ? (
          <IconButton
            tone="neutral"
            label={t("queue.markFailedSearchAgain")}
            disabled={busy}
            onClick={() => onMarkFailed(item)}
          >
            <CircleAlert className="h-3.5 w-3.5" aria-hidden="true" />
          </IconButton>
        ) : null}
        <IconButton
          tone="delete"
          label={t("queue.removeFromDownloader")}
          disabled={busy}
          onClick={() => onRemove(item)}
        >
          <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
        </IconButton>
      </span>
    </li>
  );
}

// ── Recently imported ───────────────────────────────────────────────────────

function RecentlyImportedPanel({ items }: { items: DashboardImportedItem[] }) {
  const t = useTranslate();

  return (
    <DashboardPanel
      icon={Download}
      title={t("dashboard.recentlyImported")}
      linkTo={buildViewPath("activity", undefined, undefined, undefined, undefined, "history")}
      linkLabel={t("dashboard.viewAll")}
      bodyClassName={PREVIEW_PANE_CLASS}
    >
      {items.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyImported")} />
      ) : (
        <ul>
          {items.map((item) => {
            const facet = normalizeFacet(item.facet);
            return (
              <li
                key={item.id}
                className="flex min-w-0 items-center gap-2 border-b border-border px-3 py-[7px] last:border-b-0"
              >
                <RowPoster posterUrl={item.posterUrl} facet={facet} />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[12px] font-medium text-[var(--scry-ink2)]">
                    {item.titleName ?? item.titleId}
                  </span>
                  <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-[var(--scry-muted2)]">
                    {facet ? <FacetChip facet={facet} /> : null}
                    {item.quality ? (
                      <span className="truncate">{item.quality}</span>
                    ) : null}
                    <span className="tabular-nums">{formatBytes(item.sizeBytes)}</span>
                  </span>
                </span>
                {item.eventType === "FILE_UPGRADED" ? (
                  <Badge tone="info" className="shrink-0 px-1 py-0 text-[10px]">
                    <ArrowUp className="h-2.5 w-2.5" aria-hidden="true" />
                    {t("dashboard.upgradeBadge")}
                  </Badge>
                ) : null}
                <AgeLabel isoDate={item.occurredAt} />
              </li>
            );
          })}
        </ul>
      )}
    </DashboardPanel>
  );
}

// ── Indexers ────────────────────────────────────────────────────────────────

function IndexersPanel({ overview }: { overview: DashboardOverview | null }) {
  const t = useTranslate();
  const { user } = useAuth();
  const canViewErrorHistory = hasAnyAppPermission(user, [
    APP_PERMISSIONS.manageSystemSettings,
  ]);
  const [errorHistoryIndexer, setErrorHistoryIndexer] =
    React.useState<IndexerErrorHistoryScope | null>(null);
  const indexers = React.useMemo(
    () => overview?.indexers ?? [],
    [overview?.indexers],
  );
  const statsById = React.useMemo(() => {
    const map = new Map<string, DashboardIndexerStat>();
    for (const stat of overview?.indexerStats ?? []) {
      map.set(stat.indexerId, stat);
    }
    return map;
  }, [overview?.indexerStats]);
  const health = React.useMemo(() => summarizeIndexerHealth(indexers), [indexers]);
  // Attention first, then busiest: an erroring indexer outranks a heavily used
  // healthy one, and unused healthy ones settle to the bottom.
  const sortedIndexers = React.useMemo(() => {
    const entry = (indexer: (typeof indexers)[number]) => ({
      needsAttention: isProviderErroring(
        indexer.isEnabled,
        indexer.lastHealthStatus,
        indexer.lastErrorMessage,
      ),
      usage:
        (statsById.get(indexer.id)?.queriesLast24H ?? 0) +
        (statsById.get(indexer.id)?.grabsLast24H ?? 0),
      name: indexer.name,
    });
    return [...indexers].sort((left, right) =>
      compareProviderRows(entry(left), entry(right)),
    );
  }, [indexers, statsById]);

  return (
    <DashboardPanel
      icon={Database}
      title={t("settings.indexers")}
      pills={
        <>
          <Badge tone={health.erroring > 0 ? "warning" : "positive"}>
            {t("dashboard.healthCount", {
              healthy: health.healthy,
              total: health.enabled,
            })}
          </Badge>
          {health.erroring > 0 ? (
            <Badge tone="negative">
              {health.erroring === 1
                ? t("dashboard.erroringCountOne")
                : t("dashboard.erroringCount", { count: health.erroring })}
            </Badge>
          ) : null}
          <Badge>{t("dashboard.statWindow")}</Badge>
        </>
      }
      linkTo="/integrations/indexers"
      linkLabel={t("dashboard.pluginManage")}
    >
      {indexers.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyIndexers")} />
      ) : (
        <Table
          density="dense"
          className="min-w-[440px]"
          wrapperClassName={TABLE_PANE_CLASS}
        >
          <TableHeader>
            <TableRow>
              <TableHead className={TABLE_HEAD_CLASS}>
                {t("dashboard.columnIndexer")}
              </TableHead>
              <TableHead className={TABLE_HEAD_RIGHT_CLASS}>
                {t("dashboard.columnSearch")}
              </TableHead>
              <TableHead className={TABLE_HEAD_RIGHT_CLASS}>
                {t("dashboard.columnGrab")}
              </TableHead>
              <TableHead className={TABLE_HEAD_RIGHT_CLASS}>
                {t("dashboard.columnFail")}
              </TableHead>
              <TableHead className={TABLE_HEAD_CLASS}>
                {t("dashboard.columnQuota")}
              </TableHead>
              <TableHead className={TABLE_HEAD_CLASS}>
                {t("dashboard.columnStatus")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortedIndexers.map((indexer) => {
              const stat = statsById.get(indexer.id);
              const failed = stat?.failedLast24H ?? 0;
              return (
                <TableRow key={indexer.id}>
                  <TableCell className="px-3 py-[7px]">
                    <span
                      className="block max-w-[160px] truncate text-[12px] text-[var(--scry-ink2)]"
                      title={`${indexer.name} · ${indexer.providerType}`}
                    >
                      {indexer.name}
                    </span>
                  </TableCell>
                  <TableCell className="px-3 py-[7px] text-right tabular-nums text-[var(--scry-muted)]">
                    {stat?.queriesLast24H ?? 0}
                  </TableCell>
                  <TableCell className="px-3 py-[7px] text-right tabular-nums text-[var(--scry-success-text)]">
                    {stat?.grabsLast24H ?? 0}
                  </TableCell>
                  <TableCell
                    className={cn(
                      "px-3 py-[7px] text-right tabular-nums",
                      failed > 0
                        ? "text-[var(--scry-danger-text)]"
                        : "text-[var(--scry-muted2)]",
                    )}
                  >
                    {failed}
                  </TableCell>
                  <TableCell className="px-3 py-[7px]">
                    <QuotaBar
                      current={stat?.apiCurrent ?? null}
                      max={stat?.apiMax ?? null}
                    />
                  </TableCell>
                  <TableCell className="px-3 py-[7px]">
                    <ProviderStatus
                      isEnabled={indexer.isEnabled}
                      lastHealthStatus={indexer.lastHealthStatus}
                      lastError={indexer.lastErrorMessage}
                      lastErrorAt={indexer.lastErrorAt}
                      onOpenErrorHistory={
                        canViewErrorHistory
                          ? () => setErrorHistoryIndexer({
                              id: indexer.id,
                              name: indexer.name,
                            })
                          : undefined
                      }
                    />
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}
      <IndexerErrorHistoryModal
        open={errorHistoryIndexer != null}
        onOpenChange={(open) => {
          if (!open) setErrorHistoryIndexer(null);
        }}
        indexer={errorHistoryIndexer}
      />
    </DashboardPanel>
  );
}

/** API quota as a thin bar on the shared usage ramp; unmetered shows ∞. */
function QuotaBar({ current, max }: { current: number | null; max: number | null }) {
  const t = useTranslate();
  const percent = usagePercent(current, max);

  if (percent === null) {
    return (
      <span
        className="text-[12px] text-[var(--scry-muted2)]"
        title={t("dashboard.unmetered")}
      >
        ∞
      </span>
    );
  }

  const tone = usageToneStyle(usageTone(percent).tone);
  return (
    <span className="flex items-center gap-1.5">
      <span
        className="h-1 w-14 shrink-0 overflow-hidden rounded-full"
        style={{ background: `rgba(${tone.rgb}, 0.16)` }}
      >
        <span
          className="block h-full rounded-full"
          style={{ width: `${percent}%`, background: tone.solid }}
        />
      </span>
      <span
        className="text-[11px] tabular-nums"
        style={{ color: tone.text }}
        title={`${current}/${max}`}
      >
        {Math.round(percent)}%
      </span>
    </span>
  );
}

function ProviderStatus({
  isEnabled,
  lastHealthStatus,
  lastError,
  lastErrorAt,
  onOpenErrorHistory,
}: {
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastError: string | null;
  lastErrorAt?: string | null;
  onOpenErrorHistory?: () => void;
}) {
  const t = useTranslate();

  if (!isEnabled) {
    return (
      <span className="text-[11px] text-[var(--scry-muted2)]">
        {t("label.disabled")}
      </span>
    );
  }

  if (isProviderErroring(isEnabled, lastHealthStatus, lastError)) {
    const age = formatCompactAge(lastErrorAt);
    const contents = (
      <>
        <TriangleAlert className="h-3 w-3 shrink-0" aria-hidden="true" />
        <span className="truncate">{t("dashboard.statusError")}</span>
        {age ? <span className="text-[var(--scry-muted2)]">{age}</span> : null}
      </>
    );
    if (onOpenErrorHistory) {
      return (
        <button
          type="button"
          className="flex items-center gap-1 text-left text-[11px] text-[var(--scry-danger-text)] underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          title={lastError ?? t("indexerErrors.history")}
          onClick={onOpenErrorHistory}
        >
          {contents}
        </button>
      );
    }
    return (
      <span
        className="flex items-center gap-1 text-[11px] text-[var(--scry-danger-text)]"
        title={lastError ?? t("dashboard.statusError")}
      >
        {contents}
      </span>
    );
  }

  return (
    <span className="text-[11px] text-[var(--scry-success-text)]">
      {t("dashboard.statusOk")}
    </span>
  );
}

// ── Download clients ────────────────────────────────────────────────────────

function DownloadClientsPanel({
  overview,
  queueItems,
}: {
  overview: DashboardOverview | null;
  queueItems: DownloadQueueItem[];
}) {
  const t = useTranslate();
  const clients = React.useMemo(
    () => overview?.downloadClients ?? [],
    [overview?.downloadClients],
  );
  const activity = React.useMemo(
    () => aggregateClientActivity(queueItems),
    [queueItems],
  );
  const enabled = clients.filter((client) => client.isEnabled);
  const down = enabled.filter((client) =>
    isProviderErroring(client.isEnabled, client.status, client.lastError),
  ).length;
  // Attention first, then busiest by live activity; disabled clients carry no
  // activity and no attention, so they settle to the bottom naturally.
  const sortedClients = React.useMemo(() => {
    const entry = (client: (typeof clients)[number]) => {
      const counts = activity.get(client.id);
      return {
        needsAttention: isProviderErroring(
          client.isEnabled,
          client.status,
          client.lastError,
        ),
        usage: (counts?.active ?? 0) + (counts?.queued ?? 0),
        name: client.name,
      };
    };
    return [...clients].sort((left, right) =>
      compareProviderRows(entry(left), entry(right)),
    );
  }, [activity, clients]);

  return (
    <DashboardPanel
      icon={Download}
      title={t("settings.downloadClients")}
      pills={
        <>
          <Badge tone={down > 0 ? "warning" : "positive"}>
            {t("dashboard.healthCount", {
              healthy: enabled.length - down,
              total: enabled.length,
            })}
          </Badge>
          {down > 0 ? (
            <Badge tone="negative">
              {down === 1
                ? t("dashboard.downCountOne")
                : t("dashboard.downCount", { count: down })}
            </Badge>
          ) : null}
        </>
      }
      linkTo="/integrations/download-clients"
      linkLabel={t("dashboard.pluginManage")}
    >
      {clients.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyClients")} />
      ) : (
        <Table
          density="dense"
          className="min-w-[420px]"
          wrapperClassName={TABLE_PANE_CLASS}
        >
          <TableHeader>
            <TableRow>
              <TableHead className={TABLE_HEAD_CLASS}>
                {t("dashboard.columnClient")}
              </TableHead>
              <TableHead className={TABLE_HEAD_RIGHT_CLASS}>
                {t("dashboard.columnActive")}
              </TableHead>
              <TableHead className={TABLE_HEAD_RIGHT_CLASS}>
                {t("dashboard.columnQueue")}
              </TableHead>
              <TableHead className={TABLE_HEAD_CLASS}>
                {t("dashboard.columnStatus")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortedClients.map((client) => {
              const counts = activity.get(client.id) ?? { active: 0, queued: 0 };
              return (
                <TableRow key={client.id}>
                  <TableCell className="px-3 py-[7px]">
                    <span className="flex min-w-0 items-center gap-1.5">
                      <DownloadClientTypeLogo
                        typeValue={client.clientType}
                        className="h-3.5 w-3.5 shrink-0"
                      />
                      <span
                        className="max-w-[150px] truncate text-[12px] text-[var(--scry-ink2)]"
                        title={`${client.name} · ${client.clientType}`}
                      >
                        {client.name}
                      </span>
                    </span>
                  </TableCell>
                  <TableCell
                    className={cn(
                      "px-3 py-[7px] text-right tabular-nums",
                      counts.active > 0
                        ? "text-[var(--scry-ink2)]"
                        : "text-[var(--scry-muted2)]",
                    )}
                  >
                    {counts.active}
                  </TableCell>
                  <TableCell className="px-3 py-[7px] text-right tabular-nums text-[var(--scry-muted)]">
                    {counts.queued}
                  </TableCell>
                  <TableCell className="px-3 py-[7px]">
                    {/* `lastSeenAt` is the last successful contact, not the
                        time of the error, so it is not passed as an error age. */}
                    <ProviderStatus
                      isEnabled={client.isEnabled}
                      lastHealthStatus={client.status}
                      lastError={client.lastError}
                    />
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}
    </DashboardPanel>
  );
}

// ── Storage ─────────────────────────────────────────────────────────────────

function StoragePanel({ overview }: { overview: DashboardOverview | null }) {
  const t = useTranslate();
  const roots = React.useMemo(
    () => overview?.storageRoots ?? [],
    [overview?.storageRoots],
  );
  const groups = React.useMemo(() => groupStorageRootsByLibrary(roots), [roots]);

  return (
    <DashboardPanel
      icon={HardDrive}
      title={t("dashboard.storage")}
      pills={
        <>
          <Badge className="whitespace-nowrap">
            {roots.length === 1
              ? t("dashboard.rootCountOne")
              : t("dashboard.rootCount", { count: roots.length })}
          </Badge>
        </>
      }
    >
      {roots.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyStorage")} />
      ) : (
        // Flex-wrap rather than an auto-fill grid: a grid materialises a track
        // per tile width regardless of item count, so a single-root library
        // leaves most of its row blank. Flex growth fills the last row exactly.
        <div className="flex flex-wrap gap-2 p-3">
          {groups.flatMap((group) =>
            group.roots.map((root) => (
              <StorageRootTile
                key={`${group.libraryId}:${root.path}`}
                libraryName={group.libraryName}
                root={root}
              />
            )),
          )}
        </div>
      )}
    </DashboardPanel>
  );
}

function StorageRootTile({
  libraryName,
  root,
}: {
  libraryName: string;
  root: DashboardStorageRoot;
}) {
  const t = useTranslate();
  const percent = usagePercent(root.usedBytes, root.totalBytes);
  const { tone, tag } = usageTone(percent ?? 0);
  const toneStyle = usageToneStyle(tone);
  const tagTone = percent === null ? null : usageTagBadgeTone(tag);
  const tagKey = percent === null ? null : usageTagLabelKey(tag);
  const usedTb = formatTerabytes(root.usedBytes);
  const totalTb = formatTerabytes(root.totalBytes);
  const freeTb =
    root.usedBytes !== null && root.totalBytes !== null
      ? formatTerabytes(Math.max(0, root.totalBytes - root.usedBytes))
      : null;
  const Icon = facetById(root.facet)?.icon;

  return (
    <div className="flex min-w-0 flex-[1_1_196px] items-center gap-2 rounded-lg border border-border px-2 py-1.5">
      <StorageUsageRing percent={percent} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            className="min-w-0 flex-1 truncate font-[var(--font-code)] text-[11px] text-[var(--scry-ink2)]"
            title={root.path}
          >
            {root.path}
          </span>
          {tagTone && tagKey ? (
            <Badge tone={tagTone} className="shrink-0 px-1 py-0 text-[9px]">
              {t(tagKey)}
            </Badge>
          ) : null}
          <span
            className="shrink-0 font-[var(--font-display)] text-[15px] font-bold tabular-nums"
            style={{ color: percent === null ? "var(--scry-muted2)" : toneStyle.text }}
          >
            {percent === null ? "—" : `${Math.round(percent)}%`}
          </span>
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-[10px] text-[var(--scry-muted2)]">
          <span className="flex shrink-0 items-center gap-1">
            {Icon ? (
              <Icon
                className="h-2.5 w-2.5"
                style={{ color: facetPillStyle(root.facet).color }}
                aria-hidden="true"
              />
            ) : null}
            <span className="max-w-[90px] truncate">{libraryName}</span>
          </span>
          {percent === null ? (
            <span>{t("dashboard.storageUnavailable")}</span>
          ) : (
            <>
              <span className="min-w-0 truncate tabular-nums">
                {t("dashboard.storageUsage", {
                  used: usedTb ?? "—",
                  total: totalTb ?? "—",
                })}
              </span>
              <span
                className="shrink-0 tabular-nums"
                style={
                  tag === "crit" ? { color: "var(--scry-danger-text)" } : undefined
                }
              >
                {t("dashboard.storageFree", { size: `${freeTb ?? "—"} TB` })}
              </span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Active queue ────────────────────────────────────────────────────────────

function ActiveQueuePanel({
  items,
  totalCount,
}: {
  items: DownloadQueueItem[];
  totalCount: number;
}) {
  const t = useTranslate();
  const visible = items.slice(0, QUEUE_PREVIEW_LIMIT);

  return (
    <DashboardPanel
      icon={ActivitySquare}
      title={t("dashboard.activeQueue")}
      pills={
        <Badge className="whitespace-nowrap">
          {totalCount === 1
            ? t("dashboard.queueCountOne")
            : t("dashboard.queueCount", { count: totalCount })}
        </Badge>
      }
      linkTo="/activity"
      linkLabel={t("dashboard.viewAll")}
    >
      {visible.length === 0 ? (
        <DashboardPanelEmpty message={t("dashboard.emptyQueue")} />
      ) : (
        <ul>
          {visible.map((item) => (
            <QueueRow key={item.id} item={item} />
          ))}
        </ul>
      )}
    </DashboardPanel>
  );
}

function QueueRow({ item }: { item: DownloadQueueItem }) {
  const t = useTranslate();
  // Reuses the activity view's state derivation so the dashboard can never
  // disagree with the queue page about what a row is doing.
  const presentation = deriveQueueRowPresentation(item, t);

  return (
    <li className="flex min-w-0 flex-col gap-1 border-b border-border px-3 py-[7px] last:border-b-0">
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-[var(--scry-ink2)]">
          {presentation.displayTitle}
        </span>
        <span className="shrink-0 text-[11px] text-[var(--scry-muted)]">
          {presentation.statusLabel}
        </span>
      </div>
      <ActivityProgressBar
        percent={presentation.percent}
        remainingLabel={presentation.remainingLabel}
        colorClass={getProgressBarColor(presentation.displayStateKey)}
        compact
        hideLabel
      />
      <div className="flex min-w-0 items-center gap-2 text-[10px] text-[var(--scry-muted2)]">
        <span className="tabular-nums">{formatBytes(item.sizeBytes)}</span>
        <span className="tabular-nums">{presentation.percent}%</span>
        <span className="ml-auto shrink-0 tabular-nums">
          {presentation.remainingLabel ?? "—"}
        </span>
      </div>
    </li>
  );
}

// ── Shared bits ─────────────────────────────────────────────────────────────

function FacetChip({ facet }: { facet: Facet }) {
  const t = useTranslate();
  const definition = facetById(facet);
  if (!definition) {
    return null;
  }
  const Icon = definition.icon;

  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded px-1 py-px text-[10px]"
      style={facetPillStyle(facet)}
    >
      <Icon className="h-2.5 w-2.5" aria-hidden="true" />
      {t(definition.navLabelKey)}
    </span>
  );
}

function AgeLabel({ isoDate }: { isoDate: string | null }) {
  const age = formatCompactAge(isoDate);
  return (
    <span
      className="shrink-0 whitespace-nowrap text-[11px] tabular-nums text-[var(--scry-muted2)]"
      title={isoDate ?? undefined}
    >
      {age ?? "—"}
    </span>
  );
}

function normalizeFacet(value: string | null): Facet | null {
  switch (value) {
    case "MOVIE":
    case "SERIES":
    case "ANIME":
      return value;
    default:
      return null;
  }
}

function DashboardSkeleton() {
  return (
    <div className="flex w-full flex-col gap-3 px-5 pb-10 pt-4">
      <Skeleton className="h-9 w-64" />
      <div className="grid grid-cols-1 gap-3 min-[701px]:grid-cols-2 min-[1081px]:grid-cols-3">
        <Skeleton className="h-14" />
        <Skeleton className="h-14" />
        <Skeleton className="h-14" />
      </div>
      <div className="grid grid-cols-1 gap-3 min-[1241px]:grid-cols-2 min-[1501px]:grid-cols-3">
        <Skeleton className="h-52" />
        <Skeleton className="h-52" />
        <Skeleton className="h-52" />
      </div>
      <div className="grid grid-cols-1 gap-3 min-[1241px]:grid-cols-2">
        <Skeleton className="h-48" />
        <Skeleton className="h-48" />
      </div>
    </div>
  );
}

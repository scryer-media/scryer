import * as React from "react";
import { Link } from "react-router";
import { useQuery } from "urql";
import {
  ChevronDown,
  ChevronUp,
  Loader2,
  RotateCcw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { TitleHistoryEvent } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDate, formatUiTime } from "@/lib/utils/date-format";
import {
  compareHistoryEpisodes,
  formatHistoryEpisodeLabel,
  type HistoryEpisodeDisplay,
} from "@/lib/utils/history-episodes";
import { redactHistoryApiKeys } from "@/lib/utils/history-redaction";
import { selectorId } from "@/lib/utils/dom-ids";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import type { ViewId } from "@/components/root/types";
import { HistoryEventIcon } from "./history-event-icon";
import {
  HistoryEventDetailContent,
  buildHistoryEventDetail,
} from "./history-event-detail";
import {
  getTitleHistoryEventLabel,
  getTitleHistoryEventMeta,
} from "./title-history-event-meta";

function primarySourceLabel(event: TitleHistoryEvent): string {
  return redactHistoryApiKeys(
    event.displayTitle ??
    event.sourceTitle ??
    event.sourcePath ??
    event.destPath ??
    "\u2014",
  );
}

function historySource(event: TitleHistoryEvent): { label: string; to: string } | null {
  const isGrab = event.eventType === "grabbed";
  const label = isGrab
    ? event.sourceProvider ?? event.sourceHint ?? event.sourceSystem
    : event.clientName ?? event.sourceSystem ?? event.sourceHint;
  if (!label?.trim()) {
    return null;
  }

  return {
    label: redactHistoryApiKeys(label),
    to: isGrab ? "/integrations/indexers" : "/integrations/download-clients",
  };
}

function actorLabel(event: TitleHistoryEvent): string {
  return event.actorDisplayName ?? event.actorUserId ?? event.actorKind ?? "\u2014";
}

function historyTypeLabel(event: TitleHistoryEvent, t: ReturnType<typeof useTranslate>): string {
  return event.eventType === "file_upgraded" ? t("history.upgrade") : t("history.initial");
}

function titleHistoryHref(event: TitleHistoryEvent): string | null {
  const viewByFacet: Record<string, ViewId> = {
    MOVIE: "movies",
    SERIES: "series",
    ANIME: "anime",
  };
  const view = viewByFacet[event.facet?.trim().toUpperCase() ?? ""];
  if (!view) {
    return null;
  }

  return `${buildOverviewDetailPath(view, null, null)}?id=${encodeURIComponent(event.titleId)}`;
}

function historyEpisodeHref(event: TitleHistoryEvent, episodeId: string): string | null {
  const titleHref = titleHistoryHref(event);
  return titleHref ? `${titleHref}&episodeId=${encodeURIComponent(episodeId)}` : null;
}

type HistoryEpisode = HistoryEpisodeDisplay;

function historyEpisodesQuery(episodeCount: number): string {
  const variables = Array.from(
    { length: episodeCount },
    (_, index) => `$episode${index}: ID!`,
  ).join(", ");
  const selections = Array.from(
    { length: episodeCount },
    (_, index) => `episode${index}: episode(titleId: $titleId, episodeId: $episode${index}) {
      id
      seasonNumber
      episodeNumber
      episodeLabel
      title
    }`,
  ).join("\n");
  return `query HistoryEventEpisodes($titleId: ID!, ${variables}) {${selections}\n}`;
}

function HistoryEpisodes({
  event,
  episodeIds,
  label,
}: {
  event: TitleHistoryEvent;
  episodeIds: string[];
  label: string;
}) {
  const t = useTranslate();
  const query = React.useMemo(
    () => historyEpisodesQuery(episodeIds.length),
    [episodeIds.length],
  );
  const variables = React.useMemo(
    () => ({
      titleId: event.titleId,
      ...Object.fromEntries(
        episodeIds.map((episodeId, index) => [`episode${index}`, episodeId]),
      ),
    }),
    [event.titleId, episodeIds],
  );
  const [{ data, fetching }] = useQuery<Record<string, HistoryEpisode | null>>({
    query,
    variables,
  });
  const orderedEpisodes = episodeIds
    .map((episodeId, index) => ({
      episodeId,
      episode: data?.[`episode${index}`] ?? null,
    }))
    .sort((left, right) => compareHistoryEpisodes(left.episode, right.episode));

  return (
    <div className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
      <span className="whitespace-nowrap text-muted-foreground">{label}</span>
      <div className="flex min-w-0 flex-col gap-y-1">
        {orderedEpisodes.map(({ episodeId, episode }) => {
          const href = historyEpisodeHref(event, episodeId);
          const label = fetching
            ? t("label.loading")
            : formatHistoryEpisodeLabel(episode, episodeId);
          return href ? (
            <Link
              key={episodeId}
              to={href}
              className="max-w-full truncate text-[var(--scry-accent-text)] transition-colors hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title={label}
            >
              {label}
            </Link>
          ) : (
            <span key={episodeId} className="truncate text-foreground" title={label}>
              {label}
            </span>
          );
        })}
      </div>
    </div>
  );
}

function canRetryEvent(event: TitleHistoryEvent, onRetry?: (importId: string, password?: string) => Promise<void>): boolean {
  return Boolean(
    onRetry &&
      event.importId &&
      (event.eventType === "import_failed" || event.eventType === "import_skipped"),
  );
}

export function HistoryEventTable({
  events,
  showTitle = false,
  showActor = false,
  titleNameMap,
  emptyMessage,
  onRetry,
}: {
  events: TitleHistoryEvent[];
  showTitle?: boolean;
  showActor?: boolean;
  titleNameMap?: Record<string, string>;
  emptyMessage?: string;
  onRetry?: (importId: string, password?: string) => Promise<void>;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [expandedRows, setExpandedRows] = React.useState<Record<string, boolean>>({});
  const [passwordDrafts, setPasswordDrafts] = React.useState<Record<string, string>>({});
  const [retryingId, setRetryingId] = React.useState<string | null>(null);
  const showActions = Boolean(onRetry);
  const columnCount =
    1 + // expander
    1 + // event
    (showTitle ? 1 : 0) +
    1 + // release name
    1 + // source
    1 + // type
    (showActor ? 1 : 0) +
    1 + // date
    (showActions ? 1 : 0);

  const toggleExpanded = React.useCallback((eventId: string) => {
    setExpandedRows((current) => ({
      ...current,
      [eventId]: !current[eventId],
    }));
  }, []);

  const setPasswordDraft = React.useCallback((eventId: string, value: string) => {
    setPasswordDrafts((current) => ({
      ...current,
      [eventId]: value,
    }));
  }, []);

  const handleRetry = React.useCallback(
    async (event: TitleHistoryEvent) => {
      if (!onRetry || !event.importId) {
        return;
      }

      const password = passwordDrafts[event.id]?.trim();
      if (event.retryRequiresPassword && !password) {
        return;
      }

      setRetryingId(event.id);
      try {
        await onRetry(event.importId, password || undefined);
      } finally {
        setRetryingId(null);
      }
    },
    [onRetry, passwordDrafts],
  );

  if (events.length === 0) {
    return (
      <p className="px-4 py-4 text-sm text-muted-foreground">
        {emptyMessage ?? t("history.empty")}
      </p>
    );
  }

  return (
    <div className="overflow-hidden">
      <Table overflow="clip" layout="fixed" density="dense">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10 text-center" />
            <TableHead className="w-36">{t("history.event")}</TableHead>
            {showTitle ? (
              <TableHead className="w-48">{t("history.titleColumn")}</TableHead>
            ) : null}
            <TableHead className="w-72">
              {t("queue.releaseTitle")} {t("label.name")}
            </TableHead>
            <TableHead className="w-36 text-center">{t("queue.source")}</TableHead>
            <TableHead className="w-24 text-center">{t("label.type")}</TableHead>
            {showActor ? (
              <TableHead className="w-32 text-center">{t("history.actor")}</TableHead>
            ) : null}
            <TableHead className="w-40 text-center">{t("history.date")}</TableHead>
            {showActions ? (
              <TableActionsHead className="w-32">
                {t("history.actions")}
              </TableActionsHead>
            ) : null}
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((event) => {
            const meta = getTitleHistoryEventMeta(event.eventType);
            const titleHref = titleHistoryHref(event);
            const source = historySource(event);
            const isExpanded = expandedRows[event.id] ?? false;
            const detail = buildHistoryEventDetail(event);
            const retryable = canRetryEvent(event, onRetry);
            const hasExpandableContent =
              detail.hasDetail ||
              retryable ||
              event.episodeIds.length > 0 ||
              Boolean(event.collectionId);

            return (
              <React.Fragment key={event.id}>
                <TableRow
                  id={selectorId(
                    "history-event-row",
                    event.eventType,
                    event.id,
                  )}
                >
                  <TableCell className="align-middle text-center">
                    {hasExpandableContent ? (
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border/60 bg-card/80 text-muted-foreground transition hover:text-foreground"
                        onClick={() => toggleExpanded(event.id)}
                        aria-label={
                          isExpanded
                            ? t("history.collapseDetails")
                            : t("history.expandDetails")
                        }
                      >
                        {isExpanded ? (
                          <ChevronUp className="h-4 w-4" />
                        ) : (
                          <ChevronDown className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                  </TableCell>
                  <TableCell className="align-middle">
                    <span
                      className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium ${meta.badgeClassName}`}
                    >
                      <HistoryEventIcon eventType={event.eventType} size={14} />
                      <span>{getTitleHistoryEventLabel(event.eventType, t)}</span>
                    </span>
                  </TableCell>
                  {showTitle ? (
                    <TableCell className="align-middle">
                      {titleHref ? (
                        <Link
                          to={titleHref}
                          className="block truncate text-sm font-medium text-foreground transition-colors hover:text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          title={
                            titleNameMap?.[event.titleId] ??
                            event.titleName ??
                            event.titleId
                          }
                        >
                          {titleNameMap?.[event.titleId] ??
                            event.titleName ??
                            event.titleId}
                        </Link>
                      ) : (
                        <div
                          className="truncate text-sm font-medium text-foreground"
                          title={
                            titleNameMap?.[event.titleId] ??
                            event.titleName ??
                            event.titleId
                          }
                        >
                          {titleNameMap?.[event.titleId] ??
                            event.titleName ??
                            event.titleId}
                        </div>
                      )}
                      {event.episodeIds.length > 0 ? (
                        <div className="mt-1 text-xs text-muted-foreground">
                          {event.episodeIds.length === 1
                            ? t("history.episodeCountSingle")
                            : t("history.episodeCountMultiple", {
                                count: event.episodeIds.length,
                              })}
                        </div>
                      ) : null}
                    </TableCell>
                  ) : null}
                  <TableCell className="align-middle">
                    <div
                      className="truncate text-sm text-foreground"
                      title={primarySourceLabel(event)}
                    >
                      {primarySourceLabel(event)}
                    </div>
                  </TableCell>
                  <TableCell className="align-middle text-center text-sm">
                    {source ? (
                      <Link
                        to={source.to}
                        className="inline-block max-w-full truncate text-[var(--scry-accent-text)] transition-colors hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        title={source.label}
                      >
                        {source.label}
                      </Link>
                    ) : (
                      <span className="text-muted-foreground">-</span>
                    )}
                  </TableCell>
                  <TableCell className="align-middle text-center text-sm text-muted-foreground">
                    {historyTypeLabel(event, t)}
                  </TableCell>
                  {showActor ? (
                    <TableCell
                      id={selectorId("history-event-actor", event.eventType, event.id)}
                      className="align-middle text-center text-sm text-muted-foreground"
                    >
                      {actorLabel(event)}
                    </TableCell>
                  ) : null}
                  <TableCell className="align-middle text-center text-sm text-muted-foreground">
                    <div className="font-medium text-foreground">
                      {formatUiDate(event.occurredAt ?? event.createdAt, dateTimeFormat)}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {formatUiTime(event.occurredAt ?? event.createdAt, dateTimeFormat)}
                    </div>
                  </TableCell>
                  {showActions ? (
                    <TableCell className="align-middle text-center">
                      {retryable && !event.retryRequiresPassword && !isExpanded ? (
                        <div className="flex justify-center">
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            disabled={retryingId === event.id}
                            onClick={() => void handleRetry(event)}
                          >
                            {retryingId === event.id ? (
                              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                            ) : (
                              <RotateCcw className="mr-2 h-4 w-4" />
                            )}
                            {t("importHistory.retry")}
                          </Button>
                        </div>
                      ) : null}
                    </TableCell>
                  ) : null}
                </TableRow>
                {isExpanded ? (
                  <TableRow>
                    <TableCell colSpan={columnCount} className="bg-card/30">
                      <div className="space-y-4 rounded-lg border border-border/60 bg-background/40 p-4">
                        {detail.hasDetail ? (
                          <HistoryEventDetailContent event={event} />
                        ) : null}
                        {(event.eventType === "imported" || event.eventType === "grabbed") &&
                        event.episodeIds.length > 0 ? (
                          <HistoryEpisodes
                            event={event}
                            episodeIds={[...new Set(event.episodeIds)]}
                            label={
                              event.eventType === "grabbed"
                                ? t("history.prospectiveEpisodes")
                                : t("history.imported")
                            }
                          />
                        ) : null}
                        {event.collectionId ? (
                          <div className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
                            <span className="whitespace-nowrap text-muted-foreground">
                              {t("history.collectionId")}
                            </span>
                            <span className="break-all text-foreground">
                              {event.collectionId}
                            </span>
                          </div>
                        ) : null}
                        {retryable ? (
                          <div className="space-y-2 border-t border-border/60 pt-4">
                            {event.retryRequiresPassword ? (
                              <div className="space-y-2">
                                <p className="text-xs text-muted-foreground">
                                  {t("importHistory.passwordRequired")}
                                </p>
                                <div className="flex flex-col gap-2 sm:flex-row">
                                  <Input
                                    type="password"
                                    value={passwordDrafts[event.id] ?? ""}
                                    onChange={(inputEvent) =>
                                      setPasswordDraft(event.id, inputEvent.target.value)
                                    }
                                    placeholder={t("importHistory.passwordPlaceholder")}
                                    className="sm:max-w-xs"
                                  />
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    disabled={
                                      retryingId === event.id ||
                                      !(passwordDrafts[event.id] ?? "").trim()
                                    }
                                    onClick={() => void handleRetry(event)}
                                  >
                                    {retryingId === event.id ? (
                                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                    ) : (
                                      <RotateCcw className="mr-2 h-4 w-4" />
                                    )}
                                    {t("importHistory.retryWithPassword")}
                                  </Button>
                                </div>
                              </div>
                            ) : (
                              <Button
                                type="button"
                                variant="outline"
                                size="sm"
                                disabled={retryingId === event.id}
                                onClick={() => void handleRetry(event)}
                              >
                                {retryingId === event.id ? (
                                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                ) : (
                                  <RotateCcw className="mr-2 h-4 w-4" />
                                )}
                                {t("importHistory.retry")}
                              </Button>
                            )}
                          </div>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                ) : null}
              </React.Fragment>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

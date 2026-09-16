import * as React from "react";
import { ChevronDown, ChevronUp, RefreshCcw, TextSearch } from "lucide-react";
import { useClient } from "urql";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { auditLogQuery } from "@/lib/graphql/queries";
import { CODE_FONT } from "@/lib/fonts";
import type { AuditLogEvent } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

const PAGE_SIZE = 100;
const AUDIT_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const AUDIT_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const AUDIT_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const AUDIT_PANEL_BODY_CLASS = "p-4 sm:p-5";
const AUDIT_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const AUDIT_TABLE_HEADER_CELL_CLASS =
  "font-semibold text-[var(--scry-muted3)]";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeAuditEvent(value: unknown): AuditLogEvent | null {
  if (!isRecord(value) || typeof value.eventId !== "string") {
    return null;
  }
  return {
    sequence: typeof value.sequence === "number" ? value.sequence : 0,
    eventId: value.eventId,
    occurredAt: typeof value.occurredAt === "string" ? value.occurredAt : "",
    actorKind:
      value.actorKind === "USER" ||
      value.actorKind === "ANONYMOUS" ||
      value.actorKind === "SYSTEM"
        ? value.actorKind
        : "SYSTEM",
    actorUserId: typeof value.actorUserId === "string" ? value.actorUserId : null,
    actorDisplayName:
      typeof value.actorDisplayName === "string" ? value.actorDisplayName : "System",
    titleId: typeof value.titleId === "string" ? value.titleId : null,
    facet: typeof value.facet === "string" ? value.facet : null,
    eventType: typeof value.eventType === "string" ? value.eventType : "unknown",
    streamKind:
      value.streamKind === "GLOBAL" ||
      value.streamKind === "TITLE" ||
      value.streamKind === "LIBRARY_SCAN" ||
      value.streamKind === "JOB_RUN" ||
      value.streamKind === "DOWNLOAD_QUEUE_ITEM"
        ? value.streamKind
        : "GLOBAL",
    streamId: typeof value.streamId === "string" ? value.streamId : null,
    payloadJson: value.payloadJson,
  };
}

function formatTimestamp(value: string, dateTimeFormat: UiDateTimeFormat): string {
  return formatUiDateTime(value, dateTimeFormat);
}

function formatLabel(value: string): string {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function payloadText(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export const SystemAuditContainer = React.memo(function SystemAuditContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [events, setEvents] = React.useState<AuditLogEvent[]>([]);
  const [expanded, setExpanded] = React.useState<Record<string, boolean>>({});
  const [loading, setLoading] = React.useState(false);
  const [loadingOlder, setLoadingOlder] = React.useState(false);
  const [hasMore, setHasMore] = React.useState(false);

  const fetchAuditEvents = React.useCallback(
    async (beforeSequence: number | null) => {
      const { data, error } = await client
        .query(auditLogQuery, {
          beforeSequence,
          limit: PAGE_SIZE,
        })
        .toPromise();
      if (error) throw error;
      return ((Array.isArray(data?.auditLog) ? data.auditLog : []) as unknown[])
        .map(normalizeAuditEvent)
        .filter((event): event is AuditLogEvent => event !== null);
    },
    [client],
  );

  const refreshAuditEvents = React.useCallback(async () => {
    setLoading(true);
    try {
      const nextEvents = await fetchAuditEvents(null);
      setHasMore(nextEvents.length === PAGE_SIZE);
      setEvents(nextEvents);
      setGlobalStatus(t("system.auditLoaded"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [fetchAuditEvents, setGlobalStatus, t]);

  const loadOlderAuditEvents = React.useCallback(async () => {
    if (events.length === 0) return;
    setLoadingOlder(true);
    const beforeSequence = Math.min(...events.map((event) => event.sequence));
    try {
      const nextEvents = await fetchAuditEvents(beforeSequence);
      setHasMore(nextEvents.length === PAGE_SIZE);
      setEvents((current) => {
        const seen = new Set(current.map((event) => event.eventId));
        return [
          ...current,
          ...nextEvents.filter((event) => !seen.has(event.eventId)),
        ];
      });
      setGlobalStatus(t("system.auditLoaded"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoadingOlder(false);
    }
  }, [events, fetchAuditEvents, setGlobalStatus, t]);

  React.useEffect(() => {
    void refreshAuditEvents();
  }, [refreshAuditEvents]);

  const toggleExpanded = React.useCallback((eventId: string) => {
    setExpanded((current) => ({
      ...current,
      [eventId]: !current[eventId],
    }));
  }, []);

  return (
    <div className="space-y-4">
      <section id="system-audit-section" className={AUDIT_PANEL_CLASS}>
        <div className={AUDIT_PANEL_HEADER_CLASS}>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0 space-y-1">
              <h2 className={`flex items-center gap-2 ${AUDIT_PANEL_TITLE_CLASS}`}>
                <TextSearch className="h-4 w-4 text-[var(--scry-accent-text)]" />
                {t("system.auditTitle")}
              </h2>
              <p className={`text-sm ${AUDIT_MUTED_TEXT_CLASS}`}>
                {events.length} events loaded
              </p>
            </div>
            <Button
              type="button"
              variant="primary"
              size="sm"
              className="w-full sm:w-auto"
              onClick={() => void refreshAuditEvents()}
              disabled={loading}
            >
              {loading ? (
                <LoadingMark className="h-4 w-4" />
              ) : (
                <RefreshCcw className="h-4 w-4" />
              )}
              {t("label.refresh")}
            </Button>
          </div>
        </div>

        <div className={AUDIT_PANEL_BODY_CLASS}>
          <div className="overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-bg)]">
            <Table overflow="clip" layout="fixed" density="dense">
              <TableHeader>
                <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                  <TableHead className="w-10 text-center" />
                  <TableHead className={`w-44 text-center ${AUDIT_TABLE_HEADER_CELL_CLASS}`}>{t("history.date")}</TableHead>
                  <TableHead className={`w-48 ${AUDIT_TABLE_HEADER_CELL_CLASS}`}>{t("history.event")}</TableHead>
                  <TableHead className={`w-36 text-center ${AUDIT_TABLE_HEADER_CELL_CLASS}`}>{t("history.actor")}</TableHead>
                  <TableHead className={`w-48 text-center ${AUDIT_TABLE_HEADER_CELL_CLASS}`}>{t("system.auditTarget")}</TableHead>
                  <TableHead className={AUDIT_TABLE_HEADER_CELL_CLASS}>{t("system.auditStream")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {events.length === 0 && !loading ? (
                  <TableRow>
                    <TableCell colSpan={6} className={`py-6 text-sm ${AUDIT_MUTED_TEXT_CLASS}`}>
                      {t("system.auditEmpty")}
                    </TableCell>
                  </TableRow>
                ) : null}
                {events.map((event) => {
                  const isExpanded = expanded[event.eventId] ?? false;
                  return (
                    <React.Fragment key={event.eventId}>
                      <TableRow
                        id={selectorId(
                          "system-audit-event-row",
                          event.eventType,
                          event.eventId,
                        )}
                        className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                      >
                        <TableCell className="text-center">
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-muted3)] transition hover:border-[var(--scry-bhover2)] hover:text-[var(--scry-ink2)]"
                            onClick={() => toggleExpanded(event.eventId)}
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
                        </TableCell>
                        <TableCell className={`align-top text-center text-sm ${AUDIT_MUTED_TEXT_CLASS}`}>
                          {formatTimestamp(event.occurredAt, dateTimeFormat)}
                        </TableCell>
                        <TableCell className="align-top">
                          <div
                            id={selectorId(
                              "system-audit-event-type",
                              event.eventType,
                              event.eventId,
                            )}
                            className="text-sm font-medium text-[var(--scry-ink2)]"
                          >
                            {formatLabel(event.eventType)}
                          </div>
                          <div className={`mt-1 text-xs ${AUDIT_MUTED_TEXT_CLASS}`}>
                            #{event.sequence}
                          </div>
                        </TableCell>
                        <TableCell className="align-top text-center">
                          <div
                            id={selectorId(
                              "system-audit-event-actor",
                              event.eventType,
                              event.eventId,
                            )}
                            data-audit-actor={event.actorDisplayName}
                            className="truncate text-sm text-[var(--scry-ink2)]"
                          >
                            {event.actorDisplayName}
                          </div>
                          <div className={`mt-1 text-xs ${AUDIT_MUTED_TEXT_CLASS}`}>
                            {formatLabel(event.actorKind)}
                          </div>
                        </TableCell>
                        <TableCell className={`align-top text-center text-sm ${AUDIT_MUTED_TEXT_CLASS}`}>
                          <span
                            id={selectorId(
                              "system-audit-event-target",
                              event.eventType,
                              event.eventId,
                            )}
                            data-audit-target={event.titleId ?? event.facet ?? ""}
                            className="block truncate"
                          >
                            {event.titleId ?? event.facet ?? "\u2014"}
                          </span>
                        </TableCell>
                        <TableCell className={`align-top text-sm ${AUDIT_MUTED_TEXT_CLASS}`}>
                          <span className="block truncate">
                            {event.streamKind}
                            {event.streamId ? ` / ${event.streamId}` : ""}
                          </span>
                        </TableCell>
                      </TableRow>
                      {isExpanded ? (
                        <TableRow className="border-[var(--scry-border3)]">
                          <TableCell colSpan={6} className="bg-[var(--scry-card2)] p-3">
                            <pre
                              className="max-h-[420px] overflow-auto whitespace-pre-wrap rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-bg)] p-4 text-xs text-[var(--scry-ink2)]"
                              style={{ fontFamily: CODE_FONT }}
                            >
                              {payloadText(event.payloadJson)}
                            </pre>
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </React.Fragment>
                  );
                })}
              </TableBody>
            </Table>
          </div>

          <div className="mt-4 flex justify-end">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={!hasMore || loadingOlder}
              onClick={() => void loadOlderAuditEvents()}
            >
              {loadingOlder ? <LoadingMark className="h-4 w-4" /> : null}
              {t("history.loadMore")}
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
});

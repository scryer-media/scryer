import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { TitleHistoryEvent } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  HistoryEventIcon,
  getEventTypeLabel,
  getEventTypeBadgeClass,
} from "./history-event-icon";
import { HistoryEventDetailHover } from "./history-event-detail";

function formatTimestamp(ts: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(ts));
  } catch {
    return ts;
  }
}

export function HistoryEventTable({
  events,
  showTitle,
  titleNameMap,
  emptyMessage,
}: {
  events: TitleHistoryEvent[];
  showTitle?: boolean;
  titleNameMap?: Record<string, string>;
  emptyMessage?: string;
}) {
  const t = useTranslate();

  if (events.length === 0) {
    return (
      <p className="py-4 text-sm text-muted-foreground">
        {emptyMessage ?? t("history.empty")}
      </p>
    );
  }

  return (
    <div className="overflow-x-auto">
      <Table className="min-w-[640px]">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10" />
            <TableHead className="w-28">{t("history.event")}</TableHead>
            {showTitle ? (
              <TableHead className="w-40">{t("history.titleColumn")}</TableHead>
            ) : null}
            <TableHead>{t("history.sourceTitle")}</TableHead>
            <TableHead className="w-28">{t("history.quality")}</TableHead>
            <TableHead className="w-44">{t("history.date")}</TableHead>
            <TableHead className="w-10" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((event) => (
            <TableRow key={event.id}>
              <TableCell className="pr-0">
                <HistoryEventIcon eventType={event.eventType} />
              </TableCell>
              <TableCell>
                <span
                  className={`inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium ${getEventTypeBadgeClass(event.eventType)}`}
                >
                  {getEventTypeLabel(event.eventType)}
                </span>
              </TableCell>
              {showTitle ? (
                <TableCell>
                  <span className="text-sm text-foreground">
                    {titleNameMap?.[event.titleId] ?? event.titleId}
                  </span>
                </TableCell>
              ) : null}
              <TableCell>
                {event.sourceTitle ? (
                  <p
                    className="max-w-md break-words whitespace-normal text-sm"
                    title={event.sourceTitle}
                  >
                    {event.sourceTitle}
                  </p>
                ) : (
                  <span className="text-xs text-muted-foreground/60">
                    {"\u2014"}
                  </span>
                )}
              </TableCell>
              <TableCell>
                {event.quality ? (
                  <span className="inline-flex items-center rounded border border-border/50 bg-muted/50 px-2 py-0.5 text-xs font-medium">
                    {event.quality}
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground/60">
                    {"\u2014"}
                  </span>
                )}
              </TableCell>
              <TableCell>
                <span className="text-xs text-muted-foreground">
                  {formatTimestamp(event.occurredAt)}
                </span>
              </TableCell>
              <TableCell>
                <HistoryEventDetailHover event={event} />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

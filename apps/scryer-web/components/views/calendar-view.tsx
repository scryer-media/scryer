import { useMemo, useCallback, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import FullCalendar from "@fullcalendar/react";
import dayGridPlugin from "@fullcalendar/daygrid";
import type {
  DatesSetArg,
  DayCellContentArg,
  DayHeaderContentArg,
  EventClickArg,
  EventContentArg,
  EventMountArg,
  MoreLinkContentArg,
} from "@fullcalendar/core";
import { Card, CardContent } from "@/components/ui/card";
import { useIsMobile } from "@/lib/hooks/use-mobile";

export type CalendarEpisodeItem = {
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

type CalendarViewProps = {
  episodes: CalendarEpisodeItem[];
  loading: boolean;
  onDateRangeChange: (start: string, end: string) => void;
  onEpisodeClick?: (episode: CalendarEpisodeItem) => void;
};

const FACET_COLORS: Record<string, string> = {
  anime: "#8b5cf6",
  movie: "#f59e0b",
  tv: "#3b82f6",
};

const FACET_LABELS: Record<string, string> = {
  anime: "Anime",
  movie: "Movie",
  tv: "TV",
};

function hexToRgbChannels(hex: string): string {
  const normalized = hex.replace("#", "");
  const value = normalized.length === 3
    ? normalized.split("").map((char) => `${char}${char}`).join("")
    : normalized;

  const r = Number.parseInt(value.slice(0, 2), 16);
  const g = Number.parseInt(value.slice(2, 4), 16);
  const b = Number.parseInt(value.slice(4, 6), 16);
  return `${r} ${g} ${b}`;
}

function formatEpisodeLabel(ep: CalendarEpisodeItem): string {
  const parts: string[] = [ep.titleName];
  if (ep.seasonNumber && ep.episodeNumber) {
    parts.push(`S${ep.seasonNumber}E${ep.episodeNumber}`);
  } else if (ep.episodeNumber) {
    parts.push(`E${ep.episodeNumber}`);
  }
  if (ep.episodeTitle) {
    parts.push(`- ${ep.episodeTitle}`);
  }
  return parts.join(" ");
}

function formatEpisodeBadge(ep: CalendarEpisodeItem): string | null {
  if (ep.seasonNumber && ep.episodeNumber) {
    return `S${ep.seasonNumber}E${ep.episodeNumber}`;
  }
  if (ep.episodeNumber) {
    return `E${ep.episodeNumber}`;
  }
  return ep.titleFacet === "movie" ? "Movie" : null;
}

function formatDateKey(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatTooltip(ep: CalendarEpisodeItem): string {
  const lines: string[] = [ep.titleName];
  if (ep.seasonNumber && ep.episodeNumber) {
    lines.push(`Season ${ep.seasonNumber}, Episode ${ep.episodeNumber}`);
  } else if (ep.episodeNumber) {
    lines.push(`Episode ${ep.episodeNumber}`);
  }
  if (ep.episodeTitle) {
    lines.push(ep.episodeTitle);
  }
  lines.push(`Type: ${FACET_LABELS[ep.titleFacet] ?? ep.titleFacet}`);
  if (!ep.monitored) {
    lines.push("(Not monitored)");
  }
  return lines.join("\n");
}

export function CalendarView({
  episodes,
  loading,
  onDateRangeChange,
  onEpisodeClick,
}: CalendarViewProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [facetFilter, setFacetFilter] = useState<string[]>(["anime", "movie", "tv"]);

  const filteredEpisodes = useMemo(
    () => episodes.filter((ep) => facetFilter.includes(ep.titleFacet)),
    [episodes, facetFilter],
  );

  const dayEventCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const ep of filteredEpisodes) {
      if (!ep.airDate) continue;
      counts.set(ep.airDate, (counts.get(ep.airDate) ?? 0) + 1);
    }
    return counts;
  }, [filteredEpisodes]);

  const events = useMemo(
    () =>
      filteredEpisodes
        .filter((ep) => ep.airDate)
        .map((ep) => ({
          id: ep.id,
          title: formatEpisodeLabel(ep),
          date: ep.airDate!,
          extendedProps: ep,
        })),
    [filteredEpisodes],
  );

  const handleDatesSet = (arg: DatesSetArg) => {
    const start = arg.startStr.slice(0, 10);
    const end = arg.endStr.slice(0, 10);
    onDateRangeChange(start, end);
  };

  const handleEventClick = useCallback(
    (arg: EventClickArg) => {
      const ep = arg.event.extendedProps as CalendarEpisodeItem;
      onEpisodeClick?.(ep);
    },
    [onEpisodeClick],
  );

  const handleEventDidMount = useCallback((arg: EventMountArg) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const facetColor = FACET_COLORS[ep.titleFacet] ?? "#6b7280";
    arg.el.setAttribute("title", formatTooltip(ep));
    arg.el.style.setProperty("--scryer-event-color", facetColor);
    arg.el.style.setProperty("--scryer-event-rgb", hexToRgbChannels(facetColor));
    arg.el.style.setProperty("--fc-event-text-color", "#f8fbff");
  }, []);

  const renderEventContent = useCallback((arg: EventContentArg) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const badge = formatEpisodeBadge(ep);
    const subtitle = arg.view.type === "dayGridWeek" ? ep.episodeTitle : null;

    return (
      <div className="fc-scryer-event-card">
        <div className="fc-scryer-event-row">
          <span className="fc-scryer-event-title">{ep.titleName}</span>
          {badge ? <span className="fc-scryer-event-badge">{badge}</span> : null}
        </div>
        {subtitle ? <div className="fc-scryer-event-subtitle">{subtitle}</div> : null}
      </div>
    );
  }, []);

  const renderDayHeaderContent = useCallback((arg: DayHeaderContentArg) => (
    <span className="fc-scryer-header-label">{arg.text}</span>
  ), []);

  const renderDayCellContent = useCallback((arg: DayCellContentArg) => {
    if (arg.view.type !== "dayGridMonth") {
      return (
        <div className="fc-scryer-day-chip">
          <span className="fc-scryer-day-label">{arg.dayNumberText}</span>
        </div>
      );
    }

    return (
      <div className="fc-scryer-day-chip">
        <span className="fc-scryer-day-pill">{arg.dayNumberText}</span>
      </div>
    );
  }, []);

  const renderMoreLinkContent = useCallback((arg: MoreLinkContentArg) => (
    <span className="fc-scryer-more-link-text">+{arg.num} more</span>
  ), []);

  const handleFacetChange = useCallback((values: string[]) => {
    if (values.length > 0) setFacetFilter(values);
  }, []);

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="mb-3 flex flex-wrap items-center gap-2">
          {Object.entries(FACET_LABELS).map(([facet, label]) => {
            const active = facetFilter.includes(facet);
            return (
              <button
                key={facet}
                type="button"
                onClick={() => handleFacetChange(
                  active
                    ? facetFilter.filter((f) => f !== facet)
                    : [...facetFilter, facet],
                )}
                className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-opacity ${
                  active ? "opacity-100" : "opacity-40"
                }`}
                style={{ backgroundColor: `${FACET_COLORS[facet]}22`, color: FACET_COLORS[facet] }}
              >
                <span
                  className="inline-block h-2 w-2 rounded-full"
                  style={{ backgroundColor: FACET_COLORS[facet] }}
                />
                {label}
              </button>
            );
          })}
        </div>
        {loading && (
          <p className="mb-2 text-sm text-muted-foreground">
            {t("label.loading")}
          </p>
        )}
        <div className="fc-scryer">
          <FullCalendar
            key={isMobile ? "calendar-mobile" : "calendar-desktop"}
            plugins={[dayGridPlugin]}
            initialView={isMobile ? "dayGridMonth" : "dayGridWeek"}
            events={events}
            eventClick={handleEventClick}
            eventDidMount={handleEventDidMount}
            datesSet={handleDatesSet}
            eventContent={renderEventContent}
            eventClassNames={(arg) => {
              const ep = arg.event.extendedProps as CalendarEpisodeItem;
              const classes = [
                "fc-scryer-event",
                `fc-scryer-facet-${ep.titleFacet}`,
              ];
              classes.push(ep.monitored ? "is-monitored" : "is-unmonitored");
              return classes;
            }}
            dayHeaderContent={renderDayHeaderContent}
            dayCellContent={renderDayCellContent}
            dayCellClassNames={(arg) => {
              const classes = ["fc-scryer-day-cell"];
              if (arg.isToday) classes.push("is-today");
              if (arg.isOther) classes.push("is-other");
              if ((dayEventCounts.get(formatDateKey(arg.date)) ?? 0) > 0) {
                classes.push("has-events");
              }
              if (arg.view.type === "dayGridMonth") classes.push("is-month");
              return classes;
            }}
            moreLinkClassNames={() => ["fc-scryer-more-link"]}
            moreLinkContent={renderMoreLinkContent}
            headerToolbar={
              isMobile
                ? {
                    left: "prev,next",
                    center: "title",
                    right: "today",
                  }
                : {
                    left: "prev,next today",
                    center: "title",
                    right: "dayGridMonth,dayGridWeek",
                  }
            }
            views={{
              dayGridMonth: {
                fixedWeekCount: true,
                showNonCurrentDates: true,
                dayMaxEvents: isMobile ? 2 : 3,
              },
              dayGridWeek: {
                dayMaxEvents: false,
              },
            }}
            contentHeight="auto"
            expandRows={true}
            eventDisplay="block"
            displayEventTime={false}
          />
        </div>
      </CardContent>
    </Card>
  );
}

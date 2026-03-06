import { useMemo, useCallback, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import FullCalendar from "@fullcalendar/react";
import dayGridPlugin from "@fullcalendar/daygrid";
import type { DatesSetArg, EventClickArg, EventMountArg } from "@fullcalendar/core";
import { Card, CardContent } from "@/components/ui/card";

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
  const [facetFilter, setFacetFilter] = useState<string[]>(["anime", "movie", "tv"]);

  const filteredEpisodes = useMemo(
    () => episodes.filter((ep) => facetFilter.includes(ep.titleFacet)),
    [episodes, facetFilter],
  );

  const events = useMemo(
    () =>
      filteredEpisodes
        .filter((ep) => ep.airDate)
        .map((ep) => ({
          id: ep.id,
          title: formatEpisodeLabel(ep),
          date: ep.airDate!,
          backgroundColor: FACET_COLORS[ep.titleFacet] ?? "#6b7280",
          borderColor: FACET_COLORS[ep.titleFacet] ?? "#6b7280",
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
    arg.el.setAttribute("title", formatTooltip(ep));
  }, []);

  const handleFacetChange = useCallback((values: string[]) => {
    if (values.length > 0) setFacetFilter(values);
  }, []);

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="mb-3 flex items-center gap-2">
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
            plugins={[dayGridPlugin]}
            initialView="dayGridWeek"
            events={events}
            eventClick={handleEventClick}
            eventDidMount={handleEventDidMount}
            datesSet={handleDatesSet}
            headerToolbar={{
              left: "prev,next today",
              center: "title",
              right: "dayGridMonth,dayGridWeek",
            }}
            contentHeight="auto"
            expandRows={true}
            eventDisplay="block"
            dayMaxEvents={false}
          />
        </div>
      </CardContent>
    </Card>
  );
}

import { useEffect, useMemo, useCallback, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslate } from "@/lib/context/translate-context";
import FullCalendar from "@fullcalendar/react";
import dayGridPlugin from "@fullcalendar/react/daygrid";
import classicThemePlugin from "@fullcalendar/react/themes/classic";
import "@fullcalendar/react/skeleton.css";
import "@fullcalendar/react/themes/classic/theme.css";
import "@fullcalendar/react/themes/classic/palette.css";
import type {
  DatesSetInfo,
  DayCellInfo,
  DayHeaderInfo,
  EventClickInfo,
  EventDisplayInfo,
  EventHoveringInfo,
  MoreLinkInfo,
  MountInfo,
} from "@fullcalendar/react";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import {
  WatchInMediaServerMenu,
  type MediaServerPlaybackLink,
} from "@/components/common/watch-in-media-server-menu";
import { CalendarClock, Eye, EyeOff } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LibraryRecord } from "@/lib/types";
import { artworkFallbackStyle } from "@/lib/utils/artwork-fallback";
import { buildCalendarEventHref } from "@/lib/utils/calendar-event-href";
import {
  readStoredCalendarViewMode,
  writeStoredCalendarViewMode,
} from "@/lib/utils/calendar-view-mode";
import {
  episodeAvailabilityPill,
  type EpisodeMediaAvailability,
} from "@/lib/utils/episode-media-availability";

export type CalendarEpisodeItem = {
  id: string;
  titleId: string;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  titleName: string;
  titleSlug?: string | null;
  titleFacet: string;
  seasonNumber: string | null;
  episodeNumber: string | null;
  episodeTitle: string | null;
  overview: string | null;
  imageUrl: string | null;
  airDate: string | null;
  monitored: boolean;
  playbackLinks: MediaServerPlaybackLink[];
  mediaAvailability: EpisodeMediaAvailability;
};

type CalendarMonitoringStatus = "monitored" | "unmonitored";

function CalendarMonitoringFilterControl({
  visibleStatuses,
  counts,
  onValueChange,
  className = "",
}: {
  visibleStatuses: CalendarMonitoringStatus[];
  counts: Record<CalendarMonitoringStatus, number>;
  onValueChange: (statuses: CalendarMonitoringStatus[]) => void;
  className?: string;
}) {
  return (
    <ToggleGroup
      type="multiple"
      variant="outline"
      size="sm"
      value={visibleStatuses}
      onValueChange={(values) => {
        const statuses = values.filter(
          (value): value is CalendarMonitoringStatus =>
            value === "monitored" || value === "unmonitored",
        );
        if (statuses.length > 0) onValueChange(statuses);
      }}
      aria-label="Calendar monitoring filter"
      className={`shrink-0 ${className}`}
    >
      <ToggleGroupItem
        value="monitored"
        variant="outline"
        size="sm"
        className="gap-1.5"
      >
        <Eye aria-hidden="true" className="h-3.5 w-3.5 text-[var(--scry-success-text)]" />
        <span>Monitored</span>
        <Badge tone="neutral" className="h-4 min-w-5 rounded-full px-1 py-0 text-[10px]">
          {counts.monitored}
        </Badge>
      </ToggleGroupItem>
      <ToggleGroupItem
        value="unmonitored"
        variant="outline"
        size="sm"
        className="gap-1.5"
      >
        <EyeOff aria-hidden="true" className="h-3.5 w-3.5 text-[var(--scry-danger-text)]" />
        <span>Unmonitored</span>
        <Badge tone="neutral" className="h-4 min-w-5 rounded-full px-1 py-0 text-[10px]">
          {counts.unmonitored}
        </Badge>
      </ToggleGroupItem>
    </ToggleGroup>
  );
}

type CalendarViewProps = {
  episodes: CalendarEpisodeItem[];
  loading: boolean;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (value: string[]) => void;
  onDateRangeChange: (start: string, end: string) => void;
  onEpisodeClick?: (episode: CalendarEpisodeItem) => void;
};

const FACET_COLORS: Record<string, string> = {
  anime: "var(--scry-facet-anime)",
  movie: "var(--scry-facet-movie)",
  series: "var(--scry-facet-series)",
};

const FACET_GRADIENTS: Record<string, string> = {
  anime: "var(--scry-facet-anime-grad)",
  movie: "var(--scry-facet-movie-grad)",
  series: "var(--scry-facet-series-grad)",
};

const FACET_GLOWS: Record<string, string> = {
  anime: "rgba(var(--scry-facet-anime-rgb), .7)",
  movie: "rgba(var(--scry-facet-movie-rgb), .7)",
  series: "rgba(var(--scry-facet-series-rgb), .7)",
};

const FACET_RGB: Record<string, string> = {
  anime: "var(--scry-facet-anime-rgb)",
  movie: "var(--scry-facet-movie-rgb)",
  series: "var(--scry-facet-series-rgb)",
};

// Handoff orders the filter pills Anime · Movie · Series.
const FACET_ORDER = ["anime", "movie", "series"] as const;

const FACET_LABELS: Record<string, string> = {
  anime: "Anime",
  movie: "Movie",
  series: "Series",
};

function CalendarLibraryFacetFilters({
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  facetFilter,
  onFacetFilterChange,
}: {
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (value: string[]) => void;
  facetFilter: string[];
  onFacetFilterChange: (value: string[]) => void;
}) {
  return (
    <div className="fc-scryer-calendar-filters flex w-full flex-wrap items-center gap-2 sm:w-auto sm:flex-nowrap">
      <LibraryMultiSelect
        libraries={libraries}
        selectedLibraryIds={selectedLibraryIds}
        onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
        disabled={librariesLoading || libraries.length === 0}
        triggerClassName="h-8 w-full rounded-[10px] text-[12.5px] sm:w-[150px]"
      />
      <ToggleGroup
        type="multiple"
        variant="outline"
        size="sm"
        value={facetFilter}
        onValueChange={(values) => {
          if (values.length > 0) onFacetFilterChange(values);
        }}
        aria-label="Calendar media types"
        className="w-full sm:w-auto"
      >
        {FACET_ORDER.map((facet) => {
          const active = facetFilter.includes(facet);
          const color = FACET_COLORS[facet];
          return (
            <ToggleGroupItem
              key={facet}
              value={facet}
              variant="outline"
              size="sm"
              className="flex-1 gap-1.5 sm:flex-none"
            >
              <span
                className="h-[9px] w-[9px] rounded-full"
                style={{
                  background: color,
                  opacity: active ? 1 : 0.35,
                  boxShadow: active ? `0 0 7px ${FACET_GLOWS[facet]}` : "none",
                }}
              />
              {FACET_LABELS[facet]}
            </ToggleGroupItem>
          );
        })}
      </ToggleGroup>
    </div>
  );
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

// The comp chip shows an air time next to the code badge. Only render it when
// the air date actually carries a time component (otherwise it's date-only).
function formatAirTime(airDate: string | null): string | null {
  if (!airDate || !airDate.includes("T")) return null;
  const time = airDate.slice(11, 16);
  return /^\d{2}:\d{2}$/.test(time) ? time : null;
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
  lines.push(`Library: ${ep.libraryName ?? ep.libraryId}`);
  lines.push(`Type: ${FACET_LABELS[ep.titleFacet] ?? ep.titleFacet}`);
  if (!ep.monitored) {
    lines.push("(Not monitored)");
  }
  return lines.join("\n");
}

type CalendarHoverPreview = {
  episode: CalendarEpisodeItem;
  anchor: {
    top: number;
    right: number;
    bottom: number;
    left: number;
  };
};

function formatAirDateLabel(airDate: string | null): string | null {
  if (!airDate) return null;
  const [year, month, day] = airDate.slice(0, 10).split("-").map(Number);
  if (!year || !month || !day) return airDate;
  const dateLabel = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(year, month - 1, day));
  const time = formatAirTime(airDate);
  return time ? `${dateLabel} at ${time}` : dateLabel;
}

function CalendarEventHoverCard({
  preview,
  onMouseEnter,
  onMouseLeave,
}: {
  preview: CalendarHoverPreview;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}) {
  const t = useTranslate();
  const { episode, anchor } = preview;
  const [failedImageUrl, setFailedImageUrl] = useState<string | null>(null);

  if (typeof document === "undefined") return null;

  const isMovie = episode.titleFacet === "movie";
  const fallbackTone = isMovie
    ? "MOVIE"
    : episode.titleFacet === "anime"
      ? "ANIME"
      : "SERIES";
  const gap = 12;
  const viewportPadding = 12;
  const width = Math.min(isMovie ? 360 : 380, window.innerWidth - viewportPadding * 2);
  const estimatedHeight = isMovie ? 220 : 370;
  const fitsOnRight = anchor.right + gap + width <= window.innerWidth - viewportPadding;
  const unclampedLeft = fitsOnRight ? anchor.right + gap : anchor.left - gap - width;
  const left = Math.max(
    viewportPadding,
    Math.min(unclampedLeft, window.innerWidth - width - viewportPadding),
  );
  const top = Math.max(
    viewportPadding,
    Math.min(
      anchor.top + (anchor.bottom - anchor.top - estimatedHeight) / 2,
      window.innerHeight - estimatedHeight - viewportPadding,
    ),
  );
  const badge = formatEpisodeBadge(episode);
  const availabilityPill = episodeAvailabilityPill(episode.mediaAvailability, t);
  const airDate = formatAirDateLabel(episode.airDate);

  return createPortal(
    <aside
      role="dialog"
      aria-label={`Watch options for ${episode.titleName}`}
      className={`fc-scryer-hover-card${isMovie ? " is-movie" : " is-episode"}`}
      style={{ left, top, width }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      <div
        className="fc-scryer-hover-card-image-wrap"
        style={artworkFallbackStyle(episode.id || episode.titleId, fallbackTone)}
      >
        {episode.imageUrl && failedImageUrl !== episode.imageUrl ? (
          <img
            src={episode.imageUrl}
            alt=""
            className="fc-scryer-hover-card-image"
            onError={() => setFailedImageUrl(episode.imageUrl)}
          />
        ) : null}
      </div>
      <div className="fc-scryer-hover-card-copy">
        <div className="fc-scryer-hover-card-badges">
          <span className="fc-scryer-hover-card-meta-badge">
            {FACET_LABELS[episode.titleFacet] ?? episode.titleFacet}
          </span>
          {badge ? <span className="fc-scryer-hover-card-meta-badge">{badge}</span> : null}
          {availabilityPill ? (
            <span className="fc-scryer-hover-card-meta-badge">
              {availabilityPill.label}
            </span>
          ) : null}
          {!episode.monitored ? (
            <span className="fc-scryer-hover-card-meta-badge">Unmonitored</span>
          ) : null}
        </div>
        <h3 className="fc-scryer-hover-card-title">{episode.titleName}</h3>
        {!isMovie && episode.episodeTitle ? (
          <p className="fc-scryer-hover-card-episode-title">{episode.episodeTitle}</p>
        ) : null}
        {episode.overview ? (
          <p className="fc-scryer-hover-card-overview">{episode.overview}</p>
        ) : null}
        <WatchInMediaServerMenu
          links={episode.playbackLinks}
          showLabel
          className="mt-2"
        />
        <div className="fc-scryer-hover-card-footer">
          {airDate ? (
            <span>
              <CalendarClock aria-hidden="true" />
              {airDate}
            </span>
          ) : null}
          <span>{episode.libraryName ?? episode.libraryId}</span>
        </div>
      </div>
    </aside>,
    document.body,
  );
}

export function CalendarView({
  episodes,
  loading,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  onDateRangeChange,
  onEpisodeClick,
}: CalendarViewProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [initialCalendarView] = useState(readStoredCalendarViewMode);
  const [facetFilter, setFacetFilter] = useState<string[]>(["anime", "movie", "series"]);
  const [visibleMonitoringStatuses, setVisibleMonitoringStatuses] = useState<
    CalendarMonitoringStatus[]
  >(["monitored", "unmonitored"]);
  const [hoverPreview, setHoverPreview] = useState<CalendarHoverPreview | null>(null);
  const hoverTimerRef = useRef<number | null>(null);

  const clearHoverTimer = useCallback(() => {
    if (hoverTimerRef.current !== null) {
      window.clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const scheduleHoverPreviewClose = useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = window.setTimeout(() => {
      setHoverPreview(null);
      hoverTimerRef.current = null;
    }, 180);
  }, [clearHoverTimer]);

  const handleHoverCardMouseEnter = useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  useEffect(() => () => clearHoverTimer(), [clearHoverTimer]);

  useEffect(() => {
    if (!hoverPreview) return;
    const closePreview = () => {
      setHoverPreview(null);
    };
    window.addEventListener("resize", closePreview);
    window.addEventListener("scroll", closePreview, true);
    return () => {
      window.removeEventListener("resize", closePreview);
      window.removeEventListener("scroll", closePreview, true);
    };
  }, [hoverPreview]);

  const monitoringCounts = useMemo(() => {
    const counts: Record<CalendarMonitoringStatus, number> = {
      monitored: 0,
      unmonitored: 0,
    };
    for (const episode of episodes) {
      if (!facetFilter.includes(episode.titleFacet)) continue;
      counts[episode.monitored ? "monitored" : "unmonitored"] += 1;
    }
    return counts;
  }, [episodes, facetFilter]);

  const filteredEpisodes = useMemo(
    () =>
      episodes.filter(
        (ep) =>
          facetFilter.includes(ep.titleFacet) &&
          visibleMonitoringStatuses.includes(ep.monitored ? "monitored" : "unmonitored"),
      ),
    [episodes, facetFilter, visibleMonitoringStatuses],
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
          url: buildCalendarEventHref(ep) ?? undefined,
          extendedProps: ep,
        })),
    [filteredEpisodes],
  );

  const handleDatesSet = (arg: DatesSetInfo) => {
    if (arg.view.type === "dayGridMonth" || arg.view.type === "dayGridWeek") {
      writeStoredCalendarViewMode(arg.view.type);
    }
    const start = arg.startStr.slice(0, 10);
    const end = arg.endStr.slice(0, 10);
    onDateRangeChange(start, end);
  };

  const handleEventClick = useCallback(
    (arg: EventClickInfo) => {
      if (
        !onEpisodeClick ||
        arg.jsEvent.button !== 0 ||
        arg.jsEvent.metaKey ||
        arg.jsEvent.ctrlKey ||
        arg.jsEvent.shiftKey ||
        arg.jsEvent.altKey
      ) {
        return;
      }
      arg.jsEvent.preventDefault();
      const ep = arg.event.extendedProps as CalendarEpisodeItem;
      onEpisodeClick(ep);
    },
    [onEpisodeClick],
  );

  const handleEventMouseEnter = useCallback(
    (arg: EventHoveringInfo) => {
      if (isMobile) return;
      clearHoverTimer();
      const episode = arg.event.extendedProps as CalendarEpisodeItem;
      const rect = arg.el.getBoundingClientRect();
      hoverTimerRef.current = window.setTimeout(() => {
        setHoverPreview({
          episode,
          anchor: {
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
            left: rect.left,
          },
        });
        hoverTimerRef.current = null;
      }, 180);
    },
    [clearHoverTimer, isMobile],
  );

  const handleEventMouseLeave = useCallback(() => {
    scheduleHoverPreviewClose();
  }, [scheduleHoverPreviewClose]);

  const handleEventDidMount = useCallback((arg: MountInfo<EventDisplayInfo>) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const facetColor = FACET_COLORS[ep.titleFacet] ?? "#6b7280";
    const facetGradient = FACET_GRADIENTS[ep.titleFacet] ?? facetColor;
    const facetRgb = FACET_RGB[ep.titleFacet] ?? "107, 114, 128";
    arg.el.setAttribute("aria-label", formatTooltip(ep));
    arg.el.style.setProperty("--scryer-event-color", facetColor);
    arg.el.style.setProperty("--scryer-event-accent", facetColor);
    arg.el.style.setProperty("--scryer-event-gradient", facetGradient);
    arg.el.style.setProperty("--scryer-event-rgb", facetRgb);
  }, []);

  const renderEventContent = useCallback((arg: EventDisplayInfo) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const badge = formatEpisodeBadge(ep);
    const availabilityPill = episodeAvailabilityPill(ep.mediaAvailability, t);
    const time = formatAirTime(ep.airDate);

    return (
      <div className="fc-scryer-event-card">
        <div className="fc-scryer-event-title-row">
          <span
            role="img"
            aria-label={ep.monitored ? "Monitored" : "Unmonitored"}
            className={`fc-scryer-event-monitoring-icon${
              ep.monitored ? " is-monitored" : " is-unmonitored"
            }`}
          >
            {ep.monitored ? <Eye aria-hidden="true" /> : <EyeOff aria-hidden="true" />}
          </span>
          <div className="fc-scryer-event-title">{ep.titleName}</div>
        </div>
        {badge || availabilityPill || time ? (
          <div className="fc-scryer-event-meta">
            {badge ? <span className="fc-scryer-event-badge">{badge}</span> : null}
            {availabilityPill ? (
              <span className="fc-scryer-event-badge">
                {availabilityPill.label}
              </span>
            ) : null}
            {time ? <span className="fc-scryer-event-time">{time}</span> : null}
          </div>
        ) : null}
      </div>
    );
  }, [t]);

  const renderDayHeaderContent = useCallback((arg: DayHeaderInfo) => (
    <span className="fc-scryer-header-label">{arg.text}</span>
  ), []);

  const renderDayCellContent = useCallback((arg: DayCellInfo) => {
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

  const renderMoreLinkContent = useCallback((arg: MoreLinkInfo) => (
    <span className="fc-scryer-more-link-text">+{arg.num} more</span>
  ), []);

  const handleFacetChange = useCallback((values: string[]) => {
    if (values.length > 0) setFacetFilter(values);
  }, []);

  return (
    <>
      <Card className="flex min-h-0 flex-1 flex-col rounded-none border-0 bg-transparent shadow-none">
        <CardContent className="flex min-h-0 flex-1 flex-col p-0">
          {isMobile ? (
            <div className="mb-3 flex w-full flex-col gap-2">
              <CalendarLibraryFacetFilters
                libraries={libraries}
                librariesLoading={librariesLoading}
                selectedLibraryIds={selectedLibraryIds}
                onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
                facetFilter={facetFilter}
                onFacetFilterChange={handleFacetChange}
              />
              <CalendarMonitoringFilterControl
                visibleStatuses={visibleMonitoringStatuses}
                counts={monitoringCounts}
                onValueChange={setVisibleMonitoringStatuses}
                className="w-full"
              />
            </div>
          ) : null}
        <div className="fc-scryer min-h-0 flex-1">
          <FullCalendar
            key={isMobile ? "calendar-mobile" : "calendar-desktop"}
            plugins={[dayGridPlugin, classicThemePlugin]}
            initialView={initialCalendarView}
            headerToolbarClass="fc-scryer-header-toolbar"
            toolbarSectionClass="fc-scryer-toolbar-section"
            toolbarTitleClass="fc-scryer-toolbar-title"
            buttonClass={(arg) =>
              `fc-scryer-button${arg.isSelected ? " is-selected" : ""}`
            }
            events={events}
            eventClick={handleEventClick}
            eventMouseEnter={handleEventMouseEnter}
            eventMouseLeave={handleEventMouseLeave}
            eventDidMount={handleEventDidMount}
            datesSet={handleDatesSet}
            eventContent={renderEventContent}
            eventClass={(arg) => {
              const ep = arg.event.extendedProps as CalendarEpisodeItem;
              const classes = [
                "fc-scryer-event",
                `fc-scryer-facet-${ep.titleFacet}`,
              ];
              classes.push(ep.monitored ? "is-monitored" : "is-unmonitored");
              return classes.join(" ");
            }}
            dayHeaderClass="fc-scryer-day-header"
            dayHeaderInnerClass="fc-scryer-day-header-inner"
            dayHeaderContent={renderDayHeaderContent}
            dayCellTopContent={renderDayCellContent}
            dayCellTopClass="fc-scryer-day-cell-top"
            dayCellTopInnerClass="fc-scryer-day-cell-top-inner"
            dayCellInnerClass="fc-scryer-day-cell-inner"
            dayCellBottomClass="fc-scryer-day-cell-bottom"
            dayCellClass={(arg) => {
              const classes = ["fc-scryer-day-cell"];
              if (arg.isToday) classes.push("is-today");
              if (arg.isOther) classes.push("is-other");
              if ((dayEventCounts.get(formatDateKey(arg.date)) ?? 0) > 0) {
                classes.push("has-events");
              }
              if (arg.view.type === "dayGridMonth") classes.push("is-month");
              return classes.join(" ");
            }}
            popoverClass="fc-scryer-popover"
            moreLinkClass="fc-scryer-more-link"
            moreLinkContent={renderMoreLinkContent}
            buttonGroupClass="fc-scryer-button-group"
            buttons={{
              today: { text: "Today" },
              dayGridMonth: { text: "Month" },
              dayGridWeek: { text: "Week" },
            }}
            toolbarElements={{
              calendarLibraryFacetFilters: () => (
                <CalendarLibraryFacetFilters
                  libraries={libraries}
                  librariesLoading={librariesLoading}
                  selectedLibraryIds={selectedLibraryIds}
                  onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
                  facetFilter={facetFilter}
                  onFacetFilterChange={handleFacetChange}
                />
              ),
              calendarMonitoringFilter: () => (
                <CalendarMonitoringFilterControl
                  visibleStatuses={visibleMonitoringStatuses}
                  counts={monitoringCounts}
                  onValueChange={setVisibleMonitoringStatuses}
                />
              ),
            }}
            headerToolbar={
              isMobile
                ? {
                    left: "prev,next",
                    center: "title",
                    right: "today",
                  }
                : {
                    left: "calendarLibraryFacetFilters calendarMonitoringFilter",
                    center: "title",
                    right: "today dayGridMonth,dayGridWeek prev,next",
                  }
            }
            views={{
              dayGrid: {
                className: "fc-scryer-daygrid",
                tableBodyClass: "fc-scryer-daygrid-body",
              },
              dayGridMonth: {
                fixedWeekCount: !isMobile,
                showNonCurrentDates: true,
                dayMaxEvents: isMobile ? 1 : 3,
              },
              dayGridWeek: {
                dayMaxEvents: false,
              },
            }}
            height={isMobile ? "auto" : "100%"}
            contentHeight={isMobile ? "auto" : "100%"}
            expandRows={!isMobile}
            eventDisplay="block"
            displayEventTime={false}
          />
        </div>
          {loading ? (
            <p aria-live="polite" className="mt-2 text-[12.5px] text-[var(--scry-muted3)]">
              {t("label.loading")}
            </p>
          ) : null}
        </CardContent>
      </Card>
      {!isMobile && hoverPreview ? (
        <CalendarEventHoverCard
          preview={hoverPreview}
          onMouseEnter={handleHoverCardMouseEnter}
          onMouseLeave={scheduleHoverPreviewClose}
        />
      ) : null}
    </>
  );
}

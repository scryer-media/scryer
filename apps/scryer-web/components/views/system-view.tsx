import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { ApplicationUpgradeSection } from "@/components/common/application-upgrade";
import { LazyCodeEditor } from "@/components/common/lazy-code-editor";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  CheckCircle2,
  Database,
  ExternalLink,
  Film,
  Pause,
  Play,
  Search,
  Server,
  Terminal,
  Trash2,
  Users,
  XCircle,
} from "lucide-react";
import { SingleSelectField } from "@/components/ui/select";
import "@fontsource-variable/jetbrains-mono";
import { serviceLogsQuery, serviceLogLinesSubscription } from "@/lib/graphql/queries";
import { CODE_FONT } from "@/lib/fonts";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  parseServiceLogLine,
  prettyServiceLogLine,
  type ParsedServiceLogLine,
} from "@/lib/utils/service-log-lines";

type SystemViewState = {
  systemHealth: SystemHealth | null;
  systemLoading: boolean;
  refreshSystem: () => Promise<void>;
};

type IndexerQueryStats = {
  indexerId: string;
  indexerName: string;
  queriesLast24H: number;
  successfulLast24H: number;
  failedLast24H: number;
  lastQueryAt: string | null;
  apiCurrent: number | null;
  apiMax: number | null;
  grabCurrent: number | null;
  grabMax: number | null;
};

type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  datastoreEngine: string;
  datastoreMigrationKey: string | null;
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesSeries: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
  dbMigrationVersion: string | null;
  indexerStats: IndexerQueryStats[];
};

type DataSource = {
  nameKey: string;
  href: string;
};

const DATA_SOURCES: DataSource[] = [
  { nameKey: "system.sourceTvdbName", href: "https://www.thetvdb.com/" },
  { nameKey: "system.sourceTmdbName", href: "https://www.themoviedb.org/" },
  { nameKey: "system.sourceMalName", href: "https://myanimelist.net/" },
  { nameKey: "system.sourceAniBridgeName", href: "https://github.com/anibridge/anibridge" },
];

const SYSTEM_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const SYSTEM_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const SYSTEM_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const SYSTEM_PANEL_BODY_CLASS = "p-4 sm:p-5";
const SYSTEM_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const SYSTEM_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const ignoreReadOnlyCodeChange = () => {};

function detectLogLevel(line: string): string {
  const match = String(line ?? "").match(/\b(ERROR|WARN|WARNING|INFO|DEBUG|TRACE)\b/i);
  if (!match) return "info";
  if (match[1].toLowerCase() === "warning") return "warn";
  return match[1].toLowerCase();
}

function quotaBadgeClass(current: number | null, max: number | null): string {
  if (current === null || max === null || max === 0) return "";
  const pct = current / max;
  if (pct >= 1) return "text-[var(--scry-danger-text-soft)] font-semibold";
  if (pct >= 0.9) return "text-[var(--scry-danger-text-soft)]";
  if (pct >= 0.75) return "text-[var(--scry-warning-text)]";
  return "text-[var(--scry-success-text-soft)]";
}

const LOG_LEVEL_COLORS: Record<string, string> = {
  error: "text-red-600 dark:text-red-400",
  warn: "text-amber-600 dark:text-amber-300",
  info: "text-sky-600 dark:text-sky-300",
  debug: "text-emerald-600 dark:text-emerald-300",
  trace: "text-zinc-500 dark:text-zinc-500",
};

// Tracing default format: {timestamp} {LEVEL} {target}: {message} {key=value ...}
const TRACING_LINE_RE =
  /^(\d{4}-\d{2}-\d{2}T[\d:.]+(?:Z|[+-]\d{2}:\d{2}))\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+([\w:]+):\s+(.*)/;
const KV_RE = /(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g;
const UUID_RE = /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi;

type ParsedLine = {
  timestamp: string;
  level: string;
  target: string;
  message: string;
  kvPairs: { key: string; value: string; start: number; end: number }[];
};

type RawLogLineEntry = {
  id: number;
  raw: string;
  lower: string;
  level: string;
  parsed?: ParsedLine | null;
  structured?: ParsedServiceLogLine | null;
};

type LogLineEntry = {
  id: number;
  raw: string;
  lower: string;
  level: string;
  parsed: ParsedLine | null;
  structured: ParsedServiceLogLine | null;
};

type LogViewerSnapshot = {
  lines: LogLineEntry[];
  bufferedCount: number;
  matchedCount: number;
  liveTailing: boolean;
};

function parseLine(raw: string): ParsedLine | null {
  const m = TRACING_LINE_RE.exec(raw);
  if (!m) return null;

  const body = m[4];
  const kvPairs: ParsedLine["kvPairs"] = [];
  let kv: RegExpExecArray | null;
  KV_RE.lastIndex = 0;
  while ((kv = KV_RE.exec(body)) !== null) {
    kvPairs.push({
      key: kv[1],
      value: kv[2],
      start: kv.index,
      end: kv.index + kv[0].length,
    });
  }

  return { timestamp: m[1], level: m[2], target: m[3], message: body, kvPairs };
}

function buildRawLogLineEntry(id: number, raw: string): RawLogLineEntry {
  const structured = parseServiceLogLine(raw);
  return {
    id,
    raw,
    lower: `${raw}\n${structured?.human ?? ""}`.toLowerCase(),
    level: structured?.level ?? detectLogLevel(raw),
    structured,
  };
}

function materializeLogLineEntry(entry: RawLogLineEntry): LogLineEntry {
  if (entry.structured === undefined) {
    entry.structured = parseServiceLogLine(entry.raw);
  }
  if (entry.parsed === undefined) {
    entry.parsed = entry.structured ? null : parseLine(entry.raw);
  }

  return {
    id: entry.id,
    raw: entry.raw,
    lower: entry.lower,
    level: entry.level,
    parsed: entry.parsed,
    structured: entry.structured,
  };
}

function humanLogText(value: string): React.ReactNode {
  const fragments: React.ReactNode[] = [];
  let cursor = 0;
  for (const match of value.matchAll(UUID_RE)) {
    const index = match.index ?? 0;
    if (index > cursor) fragments.push(value.slice(cursor, index));
    fragments.push(
      <em key={index} className="italic">{"<UUID>"}</em>,
    );
    cursor = index + match[0].length;
  }
  if (cursor < value.length) fragments.push(value.slice(cursor));
  return fragments.length > 0 ? fragments : value;
}

function HighlightedLine({ entry }: { entry: LogLineEntry }) {
  if (entry.structured) {
    const levelColor = LOG_LEVEL_COLORS[entry.structured.level] ?? "text-zinc-700 dark:text-zinc-300";
    return (
      <span style={{ fontFamily: CODE_FONT }}>
        <span className="text-zinc-500 dark:text-zinc-500">{entry.structured.timestamp}</span>
        {" "}
        <span className={levelColor}>{entry.structured.level.toUpperCase().padStart(5)}</span>
        {" "}
        <span className="text-zinc-600 dark:text-zinc-400">{entry.structured.target}</span>
        <span className="text-zinc-500 dark:text-zinc-500">:</span>
        {" "}
        <span className="text-zinc-700 dark:text-zinc-300">
          {humanLogText(entry.structured.human.split(": ").slice(1).join(": "))}
        </span>
      </span>
    );
  }
  const parsed = entry.parsed;
  if (!parsed) {
    return (
      <span className="text-zinc-700 dark:text-zinc-300" style={{ fontFamily: CODE_FONT }}>
        {entry.raw}
      </span>
    );
  }

  const lvl = parsed.level.toLowerCase();
  const levelColor = LOG_LEVEL_COLORS[lvl] ?? "text-zinc-700 dark:text-zinc-300";

  const fragments: React.ReactNode[] = [];
  let cursor = 0;
  for (const kv of parsed.kvPairs) {
    if (kv.start > cursor) {
      fragments.push(
        <span key={`t${cursor}`} className="text-zinc-700 dark:text-zinc-300">
          {parsed.message.slice(cursor, kv.start)}
        </span>,
      );
    }
    fragments.push(
      <span key={`k${kv.start}`}>
        <span className="text-cyan-600 dark:text-cyan-300">{kv.key}</span>
        <span className="text-zinc-500 dark:text-zinc-500">=</span>
        <span className="text-zinc-800 dark:text-zinc-100">{kv.value}</span>
      </span>,
    );
    cursor = kv.end;
  }
  if (cursor < parsed.message.length) {
    fragments.push(
      <span key={`t${cursor}`} className="text-zinc-700 dark:text-zinc-300">
        {parsed.message.slice(cursor)}
      </span>,
    );
  }

  return (
    <span style={{ fontFamily: CODE_FONT }}>
      <span className="text-zinc-500 dark:text-zinc-500">{parsed.timestamp}</span>
      {" "}
      <span className={levelColor}>{parsed.level.padStart(5)}</span>
      {" "}
      <span className="text-zinc-600 dark:text-zinc-400">{parsed.target}</span>
      <span className="text-zinc-500 dark:text-zinc-500">:</span>
      {" "}
      {fragments}
    </span>
  );
}

const RAW_BUFFER_MAX = 2000;
const LIVE_TAIL_LINES = 300;
const MAX_RENDERED_LINES = 2000;
const LOG_INGEST_BATCH_MS = 50;
const LOG_RENDER_BATCH_MS = 150;
const EMPTY_LOG_SNAPSHOT: LogViewerSnapshot = {
  lines: [],
  bufferedCount: 0,
  matchedCount: 0,
  liveTailing: false,
};

function buildLogViewerSnapshot(
  source: RawLogLineEntry[],
  query: string,
  level: string,
  paused: boolean,
): LogViewerSnapshot {
  const normalizedQuery = query.trim().toLowerCase();
  const hasFilters = normalizedQuery.length > 0 || level !== "all";

  const matching = source.filter((line) => {
    if (normalizedQuery && !line.lower.includes(normalizedQuery)) {
      return false;
    }
    if (level !== "all" && line.level !== level) {
      return false;
    }
    return true;
  });

  const liveTailing = !paused && !hasFilters && matching.length > LIVE_TAIL_LINES;
  const visible = liveTailing
    ? matching.slice(-LIVE_TAIL_LINES)
    : matching.slice(-MAX_RENDERED_LINES);

  return {
    lines: visible.map(materializeLogLineEntry),
    bufferedCount: source.length,
    matchedCount: matching.length,
    liveTailing,
  };
}

function LogViewer() {
  const client = useClient();
  const isMobile = useIsMobile();
  const [search, setSearch] = useState("");
  const [level, setLevel] = useState("all");
  const [paused, setPaused] = useState(false);
  const [snapshot, setSnapshot] = useState<LogViewerSnapshot>(EMPTY_LOG_SNAPSHOT);
  const [connected, setConnected] = useState(false);
  const [selectedLine, setSelectedLine] = useState<LogLineEntry | null>(null);
  const selectionRegionRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const pausedRef = useRef(paused);
  const searchRef = useRef(search);
  const levelRef = useRef(level);
  const nextLineIdRef = useRef(0);
  const rawBufferRef = useRef<RawLogLineEntry[]>([]);
  const pendingLinesRef = useRef<string[]>([]);
  const ingestTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const snapshotTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!selectedLine) return;

    const handlePointerDown = (event: PointerEvent) => {
      if (selectionRegionRef.current?.contains(event.target as Node)) return;
      setSelectedLine(null);
    };

    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [selectedLine]);

  const commitSnapshot = useCallback(() => {
    const nextSnapshot = buildLogViewerSnapshot(
      rawBufferRef.current,
      searchRef.current,
      levelRef.current,
      pausedRef.current,
    );

    startTransition(() => {
      setSnapshot(nextSnapshot);
    });
  }, []);

  const scheduleSnapshot = useCallback((immediate = false) => {
    if (snapshotTimerRef.current) {
      if (!immediate) {
        return;
      }
      clearTimeout(snapshotTimerRef.current);
      snapshotTimerRef.current = null;
    }

    snapshotTimerRef.current = setTimeout(() => {
      snapshotTimerRef.current = null;
      commitSnapshot();
    }, immediate ? 0 : LOG_RENDER_BATCH_MS);
  }, [commitSnapshot]);

  const flushPendingLines = useCallback(() => {
    ingestTimerRef.current = null;
    if (pendingLinesRef.current.length === 0) {
      return;
    }

    const pending = pendingLinesRef.current.splice(0, pendingLinesRef.current.length);
    const buffer = rawBufferRef.current;

    for (const line of pending) {
      const id = nextLineIdRef.current;
      nextLineIdRef.current += 1;
      buffer.push(buildRawLogLineEntry(id, line));
    }

    if (buffer.length > RAW_BUFFER_MAX) {
      buffer.splice(0, buffer.length - RAW_BUFFER_MAX);
    }

    scheduleSnapshot();
  }, [scheduleSnapshot]);

  const enqueueLine = useCallback((line: string) => {
    pendingLinesRef.current.push(line);
    if (ingestTimerRef.current) {
      return;
    }

    ingestTimerRef.current = setTimeout(flushPendingLines, LOG_INGEST_BATCH_MS);
  }, [flushPendingLines]);

  useEffect(() => {
    pausedRef.current = paused;
    if (paused && ingestTimerRef.current) {
      clearTimeout(ingestTimerRef.current);
      ingestTimerRef.current = null;
      flushPendingLines();
    }
    scheduleSnapshot(true);
  }, [flushPendingLines, paused, scheduleSnapshot]);

  useEffect(() => {
    searchRef.current = search;
    scheduleSnapshot(true);
  }, [scheduleSnapshot, search]);

  useEffect(() => {
    levelRef.current = level;
    scheduleSnapshot(true);
  }, [level, scheduleSnapshot]);

  // Initial load via query
  useEffect(() => {
    client.query(serviceLogsQuery, { limit: RAW_BUFFER_MAX }).toPromise().then(({ data }) => {
      const initial: string[] = Array.isArray(data?.serviceLogs?.lines) ? data.serviceLogs.lines : [];
      rawBufferRef.current = initial.map((line) => {
        const id = nextLineIdRef.current;
        nextLineIdRef.current += 1;
        return buildRawLogLineEntry(id, line);
      });
      scheduleSnapshot(true);
    });
  }, [client, scheduleSnapshot]);

  useDeferredWsSubscription<{ data?: { serviceLogLines?: string } }>({
    requestKey: "serviceLogLines",
    request: { query: serviceLogLinesSubscription },
    onStart() {
      setConnected(true);
    },
    onNext(result) {
      const line = result.data?.serviceLogLines;
      if (line && !pausedRef.current) {
        enqueueLine(line);
      }
    },
    onError(err) {
      console.error("[service-logs] subscription error:", err);
      setConnected(false);
    },
    onComplete() {
      setConnected(false);
    },
  });

  useEffect(
    () => () => {
      if (ingestTimerRef.current) {
        clearTimeout(ingestTimerRef.current);
      }
      if (snapshotTimerRef.current) {
        clearTimeout(snapshotTimerRef.current);
      }
      pendingLinesRef.current = [];
    },
    [],
  );

  // Auto-scroll when new lines arrive
  useEffect(() => {
    if (autoScrollRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [snapshot.lines]);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    autoScrollRef.current = atBottom;
  }, []);

  const liveTailNotice = useMemo(() => {
    if (!snapshot.liveTailing) {
      return null;
    }

    return `Live mode is showing the latest ${snapshot.lines.length} lines from ${snapshot.bufferedCount} buffered entries. Pause or filter to inspect more history.`;
  }, [snapshot.bufferedCount, snapshot.liveTailing, snapshot.lines.length]);

  const selectedJson = useMemo(
    () => prettyServiceLogLine(selectedLine?.structured ?? null),
    [selectedLine],
  );

  return (
    <section className={`${SYSTEM_PANEL_CLASS} flex min-h-0 flex-col`}>
      <div className={SYSTEM_PANEL_HEADER_CLASS}>
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0 space-y-1">
            <h2 className={`flex items-center gap-2 ${SYSTEM_PANEL_TITLE_CLASS}`}>
              <Terminal className="h-4 w-4 text-[var(--scry-accent-text)]" />
              Live service output
            </h2>
          </div>
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2.5 py-1 text-[var(--scry-ink2)]">
              <span
                className={`size-2 rounded-full ${connected ? "bg-[var(--scry-success-solid)]" : "bg-[var(--scry-danger-solid)]"}`}
              />
              {connected ? "Live" : "Disconnected"}
            </span>
            {paused ? (
              <span className="rounded-full border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2.5 py-1 text-[var(--scry-warning-text)]">
                Paused
              </span>
            ) : null}
            <span className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2.5 py-1 text-[var(--scry-muted3)]">
              {snapshot.bufferedCount} buffered
            </span>
          </div>
        </div>
      </div>
      <div className={`${SYSTEM_PANEL_BODY_CLASS} flex min-h-0 flex-col gap-4`}>
        <div className={`${SYSTEM_INSET_CLASS} grid gap-3 p-3 lg:grid-cols-[150px_minmax(220px,1fr)_auto] lg:items-end`}>
          <div className="space-y-1">
            <SingleSelectField
              label="Level"
              labelClassName={`text-xs ${SYSTEM_MUTED_TEXT_CLASS}`}
              value={level}
              onValueChange={setLevel}
              size="compact"
              chrome="toolbar"
              options={[
                { value: "all", label: "All levels" },
                { value: "error", label: "Error" },
                { value: "warn", label: "Warn" },
                { value: "info", label: "Info" },
                { value: "debug", label: "Debug" },
                { value: "trace", label: "Trace" },
              ]}
            />
          </div>
          <div className="space-y-1">
            <Label className={`text-xs ${SYSTEM_MUTED_TEXT_CLASS}`}>Search</Label>
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--scry-muted3)]" />
              <Input
                type="search"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Filter log text..."
                className="h-9 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] pl-9 text-[13px] text-[var(--scry-body)] shadow-none"
              />
            </div>
          </div>
          <div className="flex flex-wrap gap-2 lg:justify-end">
            <Button
              size="sm"
              variant={paused ? "primary" : "secondary"}
              className="h-9 rounded-[10px] px-3 text-[13px]"
              onClick={() => setPaused((p) => !p)}
            >
              {paused ? <Play className="h-3.5 w-3.5" /> : <Pause className="h-3.5 w-3.5" />}
              {paused ? "Resume" : "Pause"}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              className="h-9 rounded-[10px] px-3 text-[13px] text-[var(--scry-danger-text)] hover:text-[var(--scry-danger-text-soft)]"
              onClick={() => {
                if (ingestTimerRef.current) {
                  clearTimeout(ingestTimerRef.current);
                  ingestTimerRef.current = null;
                }
                if (snapshotTimerRef.current) {
                  clearTimeout(snapshotTimerRef.current);
                  snapshotTimerRef.current = null;
                }
                pendingLinesRef.current = [];
                rawBufferRef.current = [];
                setSelectedLine(null);
                startTransition(() => {
                  setSnapshot(EMPTY_LOG_SNAPSHOT);
                });
                autoScrollRef.current = true;
              }}
            >
              <Trash2 className="h-3.5 w-3.5" />
              Clear
            </Button>
          </div>
        </div>
        {liveTailNotice ? (
          <div className="rounded-[10px] border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-3 py-2 text-xs text-[var(--scry-info-text)]">
            {liveTailNotice}
          </div>
        ) : null}
        <div
          ref={selectionRegionRef}
          className={`grid min-h-0 gap-4 ${selectedLine ? "xl:grid-cols-[minmax(0,1fr)_minmax(320px,0.45fr)]" : ""}`}
        >
          <div className="flex flex-col overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-bg)]">
          <div className="flex items-center justify-between gap-3 border-b border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 text-xs text-[var(--scry-muted3)]">
            <span>Line</span>
            <span>
              {snapshot.lines.length} shown · {snapshot.matchedCount} matching
            </span>
          </div>
          <div
            ref={scrollRef}
            onScroll={handleScroll}
            data-code-font
            className={`overflow-y-auto text-xs leading-5 ${isMobile ? "h-[55vh] min-h-[280px]" : "h-[calc(100vh-320px)] min-h-[400px]"}`}
            style={{ fontFamily: CODE_FONT }}
          >
            {snapshot.lines.length === 0 ? (
              <div className="flex h-full min-h-[220px] items-center justify-center p-6 text-center">
                <p className={SYSTEM_MUTED_TEXT_CLASS}>No logs available yet.</p>
              </div>
            ) : (
              <div className="p-2">
                {snapshot.lines.map((line, index) => (
                  <button
                    key={line.id}
                    type="button"
                    aria-pressed={selectedLine?.id === line.id}
                    onClick={() => setSelectedLine(line)}
                    className={`group grid w-full grid-cols-[4.75ch_minmax(0,1fr)] gap-3 rounded-[7px] px-2 py-1 text-left hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-info-border-strong)] ${selectedLine?.id === line.id ? "bg-[var(--scry-hover)]" : ""}`}
                  >
                    <span className="select-none text-right tabular-nums text-[var(--scry-faint)]">
                      {index + 1}
                    </span>
                    <div
                      className="min-w-0 whitespace-pre-wrap break-words"
                      style={{ fontFamily: CODE_FONT }}
                    >
                      <HighlightedLine entry={line} />
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
        {selectedLine ? (
          <aside className="flex min-h-[240px] flex-col overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-bg)]">
            <div className="min-h-0 flex-1 overflow-auto p-3" data-code-font>
              {selectedJson ? (
                <LazyCodeEditor
                  id="service-log-event-json"
                  value={selectedJson}
                  onChange={ignoreReadOnlyCodeChange}
                  readOnly
                  language="json"
                  copyable
                  copyLabel="Copy event JSON"
                  height={isMobile ? "45vh" : "min(55vh, 640px)"}
                />
              ) : (
                <p className="text-xs text-[var(--scry-muted3)]">This legacy text log line has no JSON event payload.</p>
              )}
            </div>
          </aside>
        ) : null}
      </div>
      </div>
    </section>
  );
}

export function SystemView({
  scryerVersion,
  state,
}: {
  scryerVersion: string | null;
  state: SystemViewState;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { systemHealth } = state;
  const healthPlaceholder = "\u2014";
  const statusReady = systemHealth?.serviceReady === true;
  const facetStats: Array<[string, number | string]> = [
    ["Movies", systemHealth?.titlesMovie ?? healthPlaceholder],
    ["Series", systemHealth?.titlesSeries ?? healthPlaceholder],
    ["Anime", systemHealth?.titlesAnime ?? healthPlaceholder],
  ];
  const recentEventPreview = systemHealth?.recentEventPreview ?? [];
  const indexerStats = systemHealth?.indexerStats ?? null;

  return (
    <div className="space-y-4 text-sm">
      <ApplicationUpgradeSection />
      <section className={SYSTEM_PANEL_CLASS}>
        <div className={SYSTEM_PANEL_HEADER_CLASS}>
          <div className="space-y-1">
            <h2 className={SYSTEM_PANEL_TITLE_CLASS}>Service health</h2>
            <p className="min-h-4 text-xs font-medium text-[var(--scry-faint)]">
              {scryerVersion ? `Scryer v${scryerVersion}` : "\u00a0"}
            </p>
          </div>
        </div>
        <div className={SYSTEM_PANEL_BODY_CLASS}>
          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_360px]">
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
              <div className={`${SYSTEM_INSET_CLASS} min-h-[138px] p-4`}>
                <div className="mb-3 flex items-center gap-2">
                  <span className="flex h-8 w-8 items-center justify-center rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                    {statusReady ? (
                      <CheckCircle2 className="h-4 w-4" />
                    ) : (
                      <XCircle className="h-4 w-4" />
                    )}
                  </span>
                  <div>
                    <p className={`text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      {t("system.serviceReady")}
                    </p>
                    <p className="text-base font-semibold text-[var(--scry-ink2)]">
                      {systemHealth
                        ? statusReady
                          ? t("label.yes")
                          : t("label.no")
                        : healthPlaceholder}
                    </p>
                  </div>
                </div>
              </div>

              <div className={`${SYSTEM_INSET_CLASS} min-h-[138px] p-4`}>
                <div className="mb-3 flex items-center gap-2">
                  <span className="flex h-8 w-8 items-center justify-center rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                    <Film className="h-4 w-4" />
                  </span>
                  <div>
                    <p className={`text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      {t("system.totalTitlesLabel")}
                    </p>
                    <p className="text-base font-semibold text-[var(--scry-ink2)]">
                      {systemHealth?.totalTitles ?? healthPlaceholder}
                    </p>
                  </div>
                </div>
                <p className={`text-xs ${SYSTEM_MUTED_TEXT_CLASS}`}>
                  {t("system.monitoredTitlesLabel")}:{" "}
                  {systemHealth?.monitoredTitles ?? healthPlaceholder}
                </p>
              </div>

              <div className={`${SYSTEM_INSET_CLASS} min-h-[138px] p-4`}>
                <div className="mb-3 flex items-center gap-2">
                  <span className="flex h-8 w-8 items-center justify-center rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                    <Users className="h-4 w-4" />
                  </span>
                  <div>
                    <p className={`text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      {t("system.usersLabel")}
                    </p>
                    <p className="text-base font-semibold text-[var(--scry-ink2)]">
                      {systemHealth?.totalUsers ?? healthPlaceholder}
                    </p>
                  </div>
                </div>
              </div>

              <div className={`${SYSTEM_INSET_CLASS} min-h-[231px] p-4 sm:col-span-2 xl:col-span-3`}>
                <div className="mb-3 flex items-center gap-2">
                  <span className="flex h-8 w-8 items-center justify-center rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                    <Database className="h-4 w-4" />
                  </span>
                  <p className="font-semibold text-[var(--scry-ink2)]">Datastore</p>
                </div>
                <div className="min-w-0 space-y-3">
                  <div>
                    <p className={`text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      {t("system.dbPathLabel")}
                    </p>
                    <p className="mt-1 break-all font-[var(--font-code)] text-xs text-[var(--scry-ink2)]">
                      {systemHealth?.dbPath ?? healthPlaceholder}
                    </p>
                  </div>
                  <div className="min-w-0">
                    <p className={`text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      Migration
                    </p>
                    <div className="mt-1 grid min-w-0 gap-2">
                      <code className="block w-fit max-w-full break-all whitespace-normal rounded-[7px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-1 text-xs text-[var(--scry-ink2)]">
                        {systemHealth
                          ? (systemHealth.dbMigrationVersion ?? "unknown")
                          : healthPlaceholder}
                      </code>
                      {systemHealth?.datastoreMigrationKey ? (
                        <code className="block w-fit max-w-full break-all whitespace-normal rounded-[7px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-1 text-xs text-[var(--scry-ink2)]">
                          {systemHealth.datastoreMigrationKey}
                        </code>
                      ) : null}
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <div className={`${SYSTEM_INSET_CLASS} min-h-[380px] p-4`}>
              <p className="mb-3 font-semibold text-[var(--scry-ink2)]">
                {t("system.facetLabel")}
              </p>
              <div className="grid grid-cols-3 gap-2">
                {facetStats.map(([label, value]) => (
                  <div
                    key={label}
                    className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3"
                  >
                    <p className={`text-xs ${SYSTEM_MUTED_TEXT_CLASS}`}>{label}</p>
                    <p className="mt-1 text-lg font-semibold text-[var(--scry-ink2)]">
                      {value}
                    </p>
                  </div>
                ))}
              </div>
              <div className="mt-4 min-h-[116px]">
                {recentEventPreview.length > 0 ? (
                  <>
                    <p className={`mb-2 text-xs uppercase tracking-[0.12em] ${SYSTEM_MUTED_TEXT_CLASS}`}>
                      {t("system.recentEventSample")}
                    </p>
                    <div className="space-y-2">
                      {recentEventPreview.slice(0, 3).map((event, index) => (
                        <p
                          key={`${event}-${index}`}
                          className="rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 text-xs text-[var(--scry-ink2)]"
                        >
                          {event}
                        </p>
                      ))}
                    </div>
                  </>
                ) : null}
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className={SYSTEM_PANEL_CLASS}>
        <div className={SYSTEM_PANEL_HEADER_CLASS}>
          <h3 className={SYSTEM_PANEL_TITLE_CLASS}>Indexer Stats (Last 24h)</h3>
        </div>
        <div className={SYSTEM_PANEL_BODY_CLASS}>
          {indexerStats === null ? (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3" aria-hidden="true">
              {[0, 1, 2].map((item) => (
                <div
                  key={item}
                  className={`${SYSTEM_INSET_CLASS} min-h-[142px] p-4 opacity-70`}
                >
                  <div className="h-4 w-2/3 rounded bg-[var(--scry-border3)]" />
                  <div className="mt-3 h-3 w-1/2 rounded bg-[var(--scry-border3)]" />
                  <div className="mt-5 space-y-2">
                    <div className="h-3 w-1/3 rounded bg-[var(--scry-border3)]" />
                    <div className="h-3 w-1/2 rounded bg-[var(--scry-border3)]" />
                  </div>
                </div>
              ))}
            </div>
          ) : indexerStats.length > 0 ? (
            <div
              className={`grid gap-3 ${
                indexerStats.length === 1
                  ? "grid-cols-1"
                  : indexerStats.length === 2
                    ? "grid-cols-1 sm:grid-cols-2"
                    : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"
              }`}
            >
              {indexerStats.map((stat) => (
                <div key={stat.indexerId} className={`${SYSTEM_INSET_CLASS} p-4`}>
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="font-semibold text-[var(--scry-ink2)]">
                        {stat.indexerName}
                      </p>
                      {stat.lastQueryAt ? (
                        <p className={`mt-1 text-xs ${SYSTEM_MUTED_TEXT_CLASS}`}>
                          Last query:{" "}
                          {formatUiDateTime(stat.lastQueryAt, dateTimeFormat)}
                        </p>
                      ) : null}
                    </div>
                    <span className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-xs text-[var(--scry-muted3)]">
                      {stat.successfulLast24H}/{stat.queriesLast24H}
                    </span>
                  </div>
                  <div className="mt-3 space-y-2 text-xs">
                    <p>
                      <span className={SYSTEM_MUTED_TEXT_CLASS}>Queries:</span>{" "}
                      {stat.queriesLast24H}
                      {stat.failedLast24H > 0 && (
                        <span className="text-[var(--scry-danger-text-soft)]">
                          {" "}
                          ({stat.failedLast24H} failed)
                        </span>
                      )}
                    </p>
                    {stat.apiMax !== null && (
                      <p>
                        <span className={SYSTEM_MUTED_TEXT_CLASS}>API usage:</span>{" "}
                        <span className={quotaBadgeClass(stat.apiCurrent, stat.apiMax)}>
                          {stat.apiCurrent ?? 0}/{stat.apiMax}
                        </span>
                      </p>
                    )}
                    {stat.grabMax !== null && (
                      <p>
                        <span className={SYSTEM_MUTED_TEXT_CLASS}>Grabs:</span>{" "}
                        <span className={quotaBadgeClass(stat.grabCurrent, stat.grabMax)}>
                          {stat.grabCurrent ?? 0}/{stat.grabMax}
                        </span>
                      </p>
                    )}
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className={`${SYSTEM_INSET_CLASS} min-h-[142px] p-4 ${SYSTEM_MUTED_TEXT_CLASS}`}>
              No indexer activity in the last 24 hours.
            </div>
          )}
        </div>
      </section>

      <section className={SYSTEM_PANEL_CLASS}>
        <div className={SYSTEM_PANEL_HEADER_CLASS}>
          <div className="flex items-center gap-2">
            <Server className="h-4 w-4 text-[var(--scry-accent-text)]" />
            <h3 className={SYSTEM_PANEL_TITLE_CLASS}>{t("system.sourcesTitle")}</h3>
          </div>
        </div>
        <div className={SYSTEM_PANEL_BODY_CLASS}>
          <p className={`mb-3 text-sm ${SYSTEM_MUTED_TEXT_CLASS}`}>
            {t("system.sourcesSupport")}
          </p>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            {DATA_SOURCES.map((source) => (
              <div key={source.href} className={`${SYSTEM_INSET_CLASS} p-4`}>
                <a
                  href={source.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-2 font-semibold text-[var(--scry-accent-text)] hover:underline"
                >
                  {t(source.nameKey)}
                  <ExternalLink className="h-3.5 w-3.5" />
                </a>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}

export function SystemLogsView() {
  return <LogViewer />;
}

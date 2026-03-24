
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { serviceLogsQuery, serviceLogLinesSubscription } from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";
import { useIsMobile } from "@/lib/hooks/use-mobile";

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
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesTv: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
  dbMigrationVersion: string | null;
  dbPendingMigrations: number;
  smgCertExpiresAt: string | null;
  smgCertDaysRemaining: number | null;
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

function detectLogLevel(line: string): string {
  const match = String(line ?? "").match(/\b(ERROR|WARN|WARNING|INFO|DEBUG|TRACE)\b/i);
  if (!match) return "info";
  if (match[1].toLowerCase() === "warning") return "warn";
  return match[1].toLowerCase();
}

function quotaBadgeClass(current: number | null, max: number | null): string {
  if (current === null || max === null || max === 0) return "";
  const pct = current / max;
  if (pct >= 1) return "text-red-500 font-semibold";
  if (pct >= 0.9) return "text-red-400";
  if (pct >= 0.75) return "text-yellow-400";
  return "text-green-400";
}

const LOG_LEVEL_COLORS: Record<string, string> = {
  error: "text-red-600 dark:text-red-400",
  warn: "text-yellow-600 dark:text-yellow-400",
  info: "text-blue-600 dark:text-blue-400",
  debug: "text-emerald-600 dark:text-emerald-400",
  trace: "text-zinc-400 dark:text-zinc-500",
};

// Tracing default format: {timestamp} {LEVEL} {target}: {message} {key=value ...}
const TRACING_LINE_RE =
  /^(\d{4}-\d{2}-\d{2}T[\d:.]+Z)\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+([\w:]+):\s+(.*)/;
const KV_RE = /(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g;

type ParsedLine = {
  timestamp: string;
  level: string;
  target: string;
  message: string;
  kvPairs: { key: string; value: string; start: number; end: number }[];
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

function HighlightedLine({ line }: { line: string }) {
  const parsed = parseLine(line);
  if (!parsed) {
    return <span className="text-foreground/80">{line}</span>;
  }

  const lvl = parsed.level.toLowerCase();
  const levelColor = LOG_LEVEL_COLORS[lvl] ?? "text-foreground/80";

  const fragments: React.ReactNode[] = [];
  let cursor = 0;
  for (const kv of parsed.kvPairs) {
    if (kv.start > cursor) {
      fragments.push(
        <span key={`t${cursor}`} className="text-foreground/70">
          {parsed.message.slice(cursor, kv.start)}
        </span>,
      );
    }
    fragments.push(
      <span key={`k${kv.start}`}>
        <span className="text-cyan-600 dark:text-cyan-400">{kv.key}</span>
        <span className="text-muted-foreground">=</span>
        <span className="text-foreground/90">{kv.value}</span>
      </span>,
    );
    cursor = kv.end;
  }
  if (cursor < parsed.message.length) {
    fragments.push(
      <span key={`t${cursor}`} className="text-foreground/70">
        {parsed.message.slice(cursor)}
      </span>,
    );
  }

  return (
    <span>
      <span className="text-muted-foreground/60">{parsed.timestamp}</span>
      {" "}
      <span className={levelColor}>{parsed.level.padStart(5)}</span>
      {" "}
      <span className="text-muted-foreground">{parsed.target}</span>
      <span className="text-muted-foreground/60">:</span>
      {" "}
      {fragments}
    </span>
  );
}

const MAX_BUFFER = 2000;

function LogViewer() {
  const client = useClient();
  const isMobile = useIsMobile();
  const [search, setSearch] = useState("");
  const [level, setLevel] = useState("all");
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [connected, setConnected] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const pausedRef = useRef(paused);
  const unsubRef = useRef<(() => void) | null>(null);
  const teardownTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { pausedRef.current = paused; });

  // Initial load via query
  useEffect(() => {
    client.query(serviceLogsQuery, { limit: MAX_BUFFER }).toPromise().then(({ data }) => {
      const initial: string[] = Array.isArray(data?.serviceLogs?.lines) ? data.serviceLogs.lines : [];
      setLines(initial);
    });
  }, [client]);

  // Subscribe to live log lines via WebSocket
  useEffect(() => {
    // StrictMode re-run: cancel the pending teardown
    if (teardownTimer.current) {
      clearTimeout(teardownTimer.current);
      teardownTimer.current = null;
      return;
    }

    const unsubscribe = wsClient.subscribe(
      { query: serviceLogLinesSubscription },
      {
        next(result) {
          const line = result.data?.serviceLogLines as string | undefined;
          if (line && !pausedRef.current) {
            setLines((prev) => {
              const next = [...prev, line];
              return next.length > MAX_BUFFER ? next.slice(next.length - MAX_BUFFER) : next;
            });
          }
          setConnected(true);
        },
        error(err) {
          console.error("[service-logs] subscription error:", err);
          setConnected(false);
        },
        complete() {
          unsubRef.current = null;
          setConnected(false);
        },
      },
    );

    unsubRef.current = unsubscribe;
    setConnected(true);

    return () => {
      teardownTimer.current = setTimeout(() => {
        teardownTimer.current = null;
        unsubscribe();
        unsubRef.current = null;
        setConnected(false);
      }, 200);
    };
  }, []);

  // Auto-scroll when new lines arrive
  useEffect(() => {
    if (autoScrollRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [lines]);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    autoScrollRef.current = atBottom;
  }, []);

  const filteredLines = useMemo(() => {
    const query = search.trim().toLowerCase();
    return lines.filter((line) => {
      if (query && !line.toLowerCase().includes(query)) return false;
      if (level !== "all" && detectLogLevel(line) !== level) return false;
      return true;
    });
  }, [level, lines, search]);

  return (
    <div className="space-y-3">
      <div className="grid gap-3 sm:flex sm:flex-wrap sm:items-end">
        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Level</Label>
          <Select value={level} onValueChange={setLevel}>
            <SelectTrigger size="sm" className="w-full sm:w-[100px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All</SelectItem>
              <SelectItem value="error">Error</SelectItem>
              <SelectItem value="warn">Warn</SelectItem>
              <SelectItem value="info">Info</SelectItem>
              <SelectItem value="debug">Debug</SelectItem>
              <SelectItem value="trace">Trace</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Search</Label>
          <Input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="filter..."
            className="h-8 w-full text-sm sm:w-48"
          />
        </div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-end">
          <Button
            size="sm"
            variant="secondary"
            className="w-full sm:w-auto"
            onClick={() => setPaused((p) => !p)}
          >
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            className="w-full sm:w-auto"
            onClick={() => {
              setLines([]);
              autoScrollRef.current = true;
            }}
          >
            Clear
          </Button>
        </div>
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground sm:ml-auto">
          <span
            className={`inline-block size-2 rounded-full ${connected ? "bg-green-400" : "bg-red-400"}`}
          />
          {connected ? "Live" : "Disconnected"}
          {paused && <span className="text-yellow-400">(paused)</span>}
        </div>
      </div>
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className={`overflow-y-auto rounded-lg border border-border bg-card text-xs leading-5 ${isMobile ? "h-[55vh] min-h-[280px]" : "h-[calc(100vh-320px)] min-h-[400px]"}`}
        style={{ fontFamily: "'Fira Code', 'Fira Mono', 'JetBrains Mono', 'Source Code Pro', 'Cascadia Code', 'Consolas', monospace" }}
      >
        {filteredLines.length === 0 ? (
          <p className="p-4 text-muted-foreground">No logs available yet.</p>
        ) : (
          <div className="p-2">
            {filteredLines.map((line, i) => (
              <div key={i} className="flex hover:bg-accent/50">
                <span className="mr-3 select-none text-right text-muted-foreground/50" style={{ minWidth: "3ch" }}>
                  {i + 1}
                </span>
                <span className="break-all">
                  <HighlightedLine line={line} />
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {filteredLines.length} lines{level !== "all" ? ` (${level})` : ""}{search ? ` matching "${search}"` : ""}
      </p>
    </div>
  );
}

export function SystemView({
  state,
}: {
  state: SystemViewState;
}) {
  const t = useTranslate();
  const { systemHealth, systemLoading, refreshSystem } = state;

  return (
    <div className="space-y-4">
      {/* Service Health */}
      <Card>
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <CardTitle>{t("system.title")}</CardTitle>
            <Button
              size="sm"
              variant="secondary"
              className="w-full sm:w-auto"
              onClick={() => void refreshSystem()}
              disabled={systemLoading}
            >
              {systemLoading ? t("system.refreshing") : t("label.refresh")}
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {!systemHealth ? (
            <p className="text-sm text-muted-foreground">{t("system.notLoaded")}</p>
          ) : (
            <div className="space-y-2">
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.serviceReady")}:</span> {systemHealth.serviceReady ? t("label.yes") : t("label.no")}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.dbPathLabel")}:</span> <span className="break-all">{systemHealth.dbPath}</span>
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">Migration:</span>{" "}
                <code className="rounded bg-muted px-1 py-0.5 text-xs">
                  {systemHealth.dbMigrationVersion ?? "unknown"}
                </code>
                {systemHealth.dbPendingMigrations > 0 && (
                  <span className="ml-2 text-yellow-400">({systemHealth.dbPendingMigrations} pending)</span>
                )}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.totalTitlesLabel")}:</span> {systemHealth.totalTitles}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.monitoredTitlesLabel")}:</span> {systemHealth.monitoredTitles}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.usersLabel")}:</span> {systemHealth.totalUsers}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.facetLabel")}:</span> movie={systemHealth.titlesMovie}, tv={systemHealth.titlesTv}, anime=
                {systemHealth.titlesAnime}, other={systemHealth.titlesOther}
              </p>
            </div>
          )}
        </CardContent>
      </Card>


      {/* Indexer Stats */}
      {systemHealth && systemHealth.indexerStats.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Indexer Stats (Last 24h)</CardTitle>
          </CardHeader>
          <CardContent>
            <div className={`grid gap-3 ${systemHealth.indexerStats.length === 1 ? "grid-cols-1" : systemHealth.indexerStats.length === 2 ? "grid-cols-1 sm:grid-cols-2" : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"}`}>
              {systemHealth.indexerStats.map((stat) => (
                <div
                  key={stat.indexerId}
                  className="rounded-xl border border-border bg-card p-3 text-sm"
                >
                  <p className="font-medium">{stat.indexerName}</p>
                  <div className="mt-1 space-y-1 text-xs">
                    <p>
                      <span className="text-muted-foreground">Queries:</span>{" "}
                      {stat.queriesLast24H}
                      {stat.failedLast24H > 0 && (
                        <span className="text-red-400"> ({stat.failedLast24H} failed)</span>
                      )}
                    </p>
                    {stat.apiMax !== null && (
                      <p>
                        <span className="text-muted-foreground">API usage:</span>{" "}
                        <span className={quotaBadgeClass(stat.apiCurrent, stat.apiMax)}>
                          {stat.apiCurrent ?? 0}/{stat.apiMax}
                        </span>
                      </p>
                    )}
                    {stat.grabMax !== null && (
                      <p>
                        <span className="text-muted-foreground">Grabs:</span>{" "}
                        <span className={quotaBadgeClass(stat.grabCurrent, stat.grabMax)}>
                          {stat.grabCurrent ?? 0}/{stat.grabMax}
                        </span>
                      </p>
                    )}
                    {stat.lastQueryAt && (
                      <p>
                        <span className="text-muted-foreground">Last query:</span>{" "}
                        {new Date(stat.lastQueryAt).toLocaleString()}
                      </p>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Data Sources */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("system.sourcesTitle")}</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="mb-2 text-sm text-muted-foreground">{t("system.sourcesSupport")}</p>
          <div className="grid grid-cols-2 gap-2 text-sm">
            {DATA_SOURCES.map((source) => (
              <div key={source.href} className="rounded-xl border border-border bg-card p-3">
                <a
                  href={source.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="font-medium text-primary hover:underline"
                >
                  {t(source.nameKey)}
                </a>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      {/* Log Viewer */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Service Logs</CardTitle>
        </CardHeader>
        <CardContent>
          <LogViewer />
        </CardContent>
      </Card>
    </div>
  );
}

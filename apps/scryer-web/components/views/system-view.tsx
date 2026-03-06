
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
import { Separator } from "@/components/ui/separator";
import { serviceLogsQuery, serviceLogLinesSubscription } from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";

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
  descriptionKey: string;
  href: string;
};

const DATA_SOURCES: DataSource[] = [
  {
    nameKey: "system.sourceTvdbName",
    descriptionKey: "system.sourceTvdbDescription",
    href: "https://www.thetvdb.com/",
  },
  {
    nameKey: "system.sourceTmdbName",
    descriptionKey: "system.sourceTmdbDescription",
    href: "https://www.themoviedb.org/",
  },
  {
    nameKey: "system.sourceJikanName",
    descriptionKey: "system.sourceJikanDescription",
    href: "https://jikan.moe/",
  },
  {
    nameKey: "system.sourceMalName",
    descriptionKey: "system.sourceMalDescription",
    href: "https://myanimelist.net/",
  },
  {
    nameKey: "system.sourceAniBridgeName",
    descriptionKey: "system.sourceAniBridgeDescription",
    href: "https://github.com/anibridge/anibridge",
  },
];

function detectLogLevel(line: string): string {
  const match = String(line ?? "").match(/\b(ERROR|WARN|WARNING|INFO|DEBUG|TRACE)\b/i);
  if (!match) return "info";
  if (match[1].toLowerCase() === "warning") return "warn";
  return match[1].toLowerCase();
}

function certBadgeClass(daysRemaining: number | null): string {
  if (daysRemaining === null) return "text-muted-foreground";
  if (daysRemaining < 0) return "text-red-500 font-semibold";
  if (daysRemaining <= 7) return "text-red-400";
  if (daysRemaining <= 30) return "text-yellow-400";
  return "text-green-400";
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
  error: "text-red-400",
  warn: "text-yellow-400",
  info: "text-blue-400",
  debug: "text-emerald-400",
  trace: "text-zinc-500",
};

const MAX_BUFFER = 2000;

function LogViewer() {
  const client = useClient();
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
      <div className="flex flex-wrap items-center gap-3">
        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Level</Label>
          <Select value={level} onValueChange={setLevel}>
            <SelectTrigger size="sm" className="w-[100px]">
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
            className="h-8 w-48 text-sm"
          />
        </div>
        <div className="flex items-end gap-2 self-end">
          <Button size="sm" variant="secondary" onClick={() => setPaused((p) => !p)}>
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setLines([]);
              autoScrollRef.current = true;
            }}
          >
            Clear
          </Button>
        </div>
        <div className="ml-auto flex items-center gap-1.5 self-end text-xs text-muted-foreground">
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
        className="h-[calc(100vh-320px)] min-h-[400px] overflow-y-auto rounded-lg border border-border bg-[#0a0e1a] text-xs leading-5"
        style={{ fontFamily: "'Fira Code', 'Fira Mono', 'JetBrains Mono', 'Source Code Pro', 'Cascadia Code', 'Consolas', monospace" }}
      >
        {filteredLines.length === 0 ? (
          <p className="p-4 text-muted-foreground">No logs available yet.</p>
        ) : (
          <div className="p-2">
            {filteredLines.map((line, i) => {
              const lvl = detectLogLevel(line);
              return (
                <div key={i} className="flex hover:bg-white/5">
                  <span className="mr-3 select-none text-right text-zinc-600" style={{ minWidth: "3ch" }}>
                    {i + 1}
                  </span>
                  <span className={`break-all ${LOG_LEVEL_COLORS[lvl] ?? "text-zinc-300"}`}>{line}</span>
                </div>
              );
            })}
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
          <div className="flex items-center justify-between">
            <CardTitle>{t("system.title")}</CardTitle>
            <Button size="sm" variant="secondary" onClick={() => void refreshSystem()} disabled={systemLoading}>
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
                {t("system.serviceReady")}: {systemHealth.serviceReady ? t("label.yes") : t("label.no")}
              </p>
              <p className="text-sm">
                {t("system.dbPathLabel")}: {systemHealth.dbPath}
              </p>
              <p className="text-sm">
                {t("system.totalTitlesLabel")}: {systemHealth.totalTitles}
              </p>
              <p className="text-sm">
                {t("system.monitoredTitlesLabel")}: {systemHealth.monitoredTitles}
              </p>
              <p className="text-sm">
                {t("system.usersLabel")}: {systemHealth.totalUsers}
              </p>
              <p className="text-sm">
                {t("system.facetLabel")}: movie={systemHealth.titlesMovie}, tv={systemHealth.titlesTv}, anime=
                {systemHealth.titlesAnime}, other={systemHealth.titlesOther}
              </p>
              <Separator />
              <p className="text-sm">{t("system.recentEventsLabel")}</p>
              <ul className="space-y-1 text-sm text-card-foreground">
                {systemHealth.recentEventPreview.map((entry) => (
                  <li key={entry} className="rounded-xl border border-border bg-card p-2">
                    {entry}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Database & SMG Certificate */}
      {systemHealth && (
        <div className="grid gap-4 md:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Database</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <p>
                <span className="text-muted-foreground">Migration version:</span>{" "}
                <code className="rounded bg-muted px-1 py-0.5 text-xs">
                  {systemHealth.dbMigrationVersion ?? "unknown"}
                </code>
              </p>
              <p>
                <span className="text-muted-foreground">Pending migrations:</span>{" "}
                <span className={systemHealth.dbPendingMigrations > 0 ? "text-yellow-400" : ""}>
                  {systemHealth.dbPendingMigrations}
                </span>
              </p>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-base">SMG Client Certificate</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              {systemHealth.smgCertExpiresAt ? (
                <>
                  <p>
                    <span className="text-muted-foreground">Expires:</span>{" "}
                    {new Date(systemHealth.smgCertExpiresAt).toLocaleString()}
                  </p>
                  <p>
                    <span className="text-muted-foreground">Days remaining:</span>{" "}
                    <span className={certBadgeClass(systemHealth.smgCertDaysRemaining)}>
                      {systemHealth.smgCertDaysRemaining !== null
                        ? systemHealth.smgCertDaysRemaining < 0
                          ? `expired ${Math.abs(systemHealth.smgCertDaysRemaining)}d ago`
                          : `${systemHealth.smgCertDaysRemaining}d`
                        : "—"}
                    </span>
                  </p>
                </>
              ) : (
                <p className="text-muted-foreground">Not enrolled</p>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Indexer Stats */}
      {systemHealth && systemHealth.indexerStats.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Indexer Stats (Last 24h)</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              {systemHealth.indexerStats.map((stat) => (
                <div
                  key={stat.indexerId}
                  className="rounded-xl border border-border bg-card p-3 text-sm"
                >
                  <p className="font-medium">{stat.indexerName}</p>
                  <div className="mt-1 grid gap-x-6 gap-y-1 text-xs sm:grid-cols-2 lg:grid-cols-3">
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
          <ul className="space-y-2 text-sm">
            {DATA_SOURCES.map((source) => (
              <li key={source.href} className="rounded-xl border border-border bg-card p-3">
                <p className="font-medium">{t(source.nameKey)}</p>
                <p className="text-xs text-muted-foreground">{t(source.descriptionKey)}</p>
                <a
                  href={source.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-xs font-medium text-primary hover:underline"
                >
                  {source.href}
                </a>
              </li>
            ))}
          </ul>
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

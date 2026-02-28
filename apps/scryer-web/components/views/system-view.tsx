
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { LazyLog, ScrollFollow } from "@melloware/react-logviewer";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type SystemViewState = {
  t: Translate;
  systemHealth: SystemHealth | null;
  systemLoading: boolean;
  refreshSystem: () => Promise<void>;
};

type IndexerQueryStats = {
  indexerId: string;
  indexerName: string;
  queriesLast24h: number;
  successfulLast24h: number;
  failedLast24h: number;
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
    nameKey: "system.sourcePlexAniBridgeName",
    descriptionKey: "system.sourcePlexAniBridgeDescription",
    href: "https://github.com/eliasbenb/PlexAniBridge-Mappings",
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

function LogViewer() {
  const [limit, setLimit] = useState("2000");
  const [search, setSearch] = useState("");
  const [level, setLevel] = useState("");
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [status, setStatus] = useState("Loading...");

  const refreshLogs = useCallback(async () => {
    const parsedLimit = Number(limit || 250);
    const safeLimit = Number.isFinite(parsedLimit) ? Math.min(Math.max(parsedLimit, 20), 2000) : 250;
    const resp = await fetch(`/api/logs?limit=${safeLimit}`, { credentials: "same-origin" });
    if (!resp.ok) {
      setStatus(`Error: ${resp.status} ${resp.statusText}`);
      return;
    }
    const result = await resp.json();
    const nextLines: string[] = Array.isArray(result.lines) ? result.lines : [];
    setLines((prev) => {
      if (prev.length === nextLines.length && prev.every((line, i) => line === nextLines[i])) return prev;
      return nextLines;
    });
    setStatus(`${result.count ?? 0} entries at ${result.generated_at ?? "unknown"}`);
  }, [limit]);

  useEffect(() => {
    refreshLogs().catch((err) => setStatus(`Refresh failed: ${err instanceof Error ? err.message : String(err)}`));
  }, [refreshLogs]);

  useEffect(() => {
    const id = window.setInterval(() => {
      if (paused) return;
      refreshLogs().catch((err) => setStatus(`Refresh failed: ${err instanceof Error ? err.message : String(err)}`));
    }, 2500);
    return () => window.clearInterval(id);
  }, [paused, refreshLogs]);

  const filteredText = useMemo(() => {
    const query = search.trim().toLowerCase();
    return lines
      .filter((line) => {
        const raw = String(line);
        if (query && !raw.toLowerCase().includes(query)) return false;
        if (level && detectLogLevel(raw) !== level) return false;
        return true;
      })
      .join("\n");
  }, [level, lines, search]);

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end gap-3">
        <label className="space-y-1 text-sm">
          <span className="text-muted-foreground">Max lines</span>
          <input
            type="number"
            min={20}
            max={2000}
            value={limit}
            onChange={(e) => setLimit(e.target.value)}
            className="block w-24 rounded-md border border-border bg-background px-2 py-1 text-sm"
          />
        </label>
        <label className="space-y-1 text-sm">
          <span className="text-muted-foreground">Level</span>
          <select
            value={level}
            onChange={(e) => setLevel(e.target.value)}
            className="block rounded-md border border-border bg-background px-2 py-1 text-sm"
          >
            <option value="">All</option>
            <option value="error">Error</option>
            <option value="warn">Warn</option>
            <option value="info">Info</option>
            <option value="debug">Debug</option>
            <option value="trace">Trace</option>
          </select>
        </label>
        <label className="space-y-1 text-sm">
          <span className="text-muted-foreground">Search</span>
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="filter..."
            className="block w-48 rounded-md border border-border bg-background px-2 py-1 text-sm"
          />
        </label>
        <Button size="sm" variant="secondary" onClick={() => void refreshLogs()}>
          Refresh
        </Button>
        <Button size="sm" variant="secondary" onClick={() => setPaused((p) => !p)}>
          {paused ? "Resume" : "Pause"}
        </Button>
      </div>
      <div className="overflow-hidden rounded-lg border border-border">
        <ScrollFollow
          startFollowing
          render={({ follow, onScroll }) => (
            <LazyLog
              text={filteredText || "No logs available yet."}
              follow={follow && !paused}
              onScroll={onScroll}
              enableSearch
              caseInsensitive
              selectableLines
              extraLines={1}
              height={560}
            />
          )}
        />
      </div>
      <p className="text-xs text-muted-foreground">{status}</p>
    </div>
  );
}

export function SystemView({
  state,
}: {
  state: SystemViewState;
}) {
  const { t, systemHealth, systemLoading, refreshSystem } = state;

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
                      {stat.queriesLast24h}
                      {stat.failedLast24h > 0 && (
                        <span className="text-red-400"> ({stat.failedLast24h} failed)</span>
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
